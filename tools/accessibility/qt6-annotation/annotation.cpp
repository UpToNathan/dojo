// Copyright (C) 2016 The Qt Company Ltd.
// Copyright (C) 2026 Nicolas Fella <nicolas.fella@kde.org>
// SPDX-License-Identifier: LGPL-3.0-only
#include "annotation-client.h"

#include <QAccessible>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QEvent>
#include <QGenericPlugin>
#include <QGuiApplication>
#include <QHash>
#include <QLibraryInfo>
#include <QPlatformSurfaceEvent>
#include <QPointer>
#include <QThread>
#include <QTimer>
#include <QVariant>
#include <QWaylandClientExtension>
#include <QWindow>
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
#include <QtGui/private/qspiaccessiblebridge_p.h>
#else
#include <dlfcn.h>
#endif
#include <QtGui/qpa/qplatformnativeinterface.h>
#include <QtWaylandClient/private/qwaylandwindow_p.h>

#include <optional>

using QtWaylandClient::QWaylandWindow;

namespace {
constexpr auto interfaceName = "org.a11y.atspi.Accessible";

QString runtimeLibrariesPath()
{
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    return QLibraryInfo::path(QLibraryInfo::LibrariesPath);
#else
    return QLibraryInfo::location(QLibraryInfo::LibrariesPath);
#endif
}

std::optional<QDBusConnection> bridgeConnection()
{
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    auto *bridge = QSpiAccessibleBridge::instance();
    if (!bridge || !bridge->isActive())
        return std::nullopt;
    return bridge->dBusConnection();
#else
    using Accessor = bool (*)(QDBusConnection *);
    auto accessor = reinterpret_cast<Accessor>(dlsym(RTLD_DEFAULT, "dojo_qt51519_accessibility_connection"));
    QDBusConnection result(QStringLiteral("dojo-unavailable"));
    if (!accessor || !accessor(&result))
        return std::nullopt;
    return result;
#endif
}

class Manager final : public QWaylandClientExtension
{
public:
    Manager() : QWaylandClientExtension(1) {}
    ~Manager() override
    {
        if (proxy)
            xdg_dbus_annotation_manager_v1_destroy(proxy);
    }
    const wl_interface *extensionInterface() const override
    {
        return &xdg_dbus_annotation_manager_v1_interface;
    }
    void bind(wl_registry *registry, int id, int version) override
    {
        setVersion(qMin(version, 1));
        proxy = static_cast<xdg_dbus_annotation_manager_v1 *>(wl_registry_bind(
            registry, id, &xdg_dbus_annotation_manager_v1_interface, this->version()));
    }
    xdg_dbus_annotation_manager_v1 *proxy = nullptr;
};

struct Endpoint {
    QString owner;
    QString path;
    wl_surface *surface;
    friend bool operator==(const Endpoint &, const Endpoint &) = default;
};

class Binding final : public QObject
{
public:
    Binding(QWindow *window, Manager *manager, QObject *parent)
        : QObject(parent), window(window), manager(manager)
    {
    }
    ~Binding() override { revoke(); }

    void revoke()
    {
        ++generation;
        pending = false;
        endpoint.reset();
        if (annotation) {
            xdg_dbus_annotation_v1_destroy(annotation);
            annotation = nullptr;
        }
    }

    void refresh()
    {
        Q_ASSERT(QThread::currentThread() == qGuiApp->thread());
        auto current = resolve();
        if (!current) {
            revoke();
            return;
        }
        if (endpoint == current && (annotation || pending))
            return;
        revoke();
        endpoint = current;
        pending = true;
        const auto requestGeneration = generation;
        const auto expected = *current;
        auto connection = bridgeConnection();
        if (!connection) {
            revoke();
            return;
        }
        auto request = QDBusMessage::createMethodCall(
            expected.owner, expected.path, QString::fromLatin1(interfaceName), QStringLiteral("GetRole"));
        auto *watcher = new QDBusPendingCallWatcher(
            connection->asyncCall(request, 500), this);
        connect(watcher, &QDBusPendingCallWatcher::finished, this,
                [this, watcher, requestGeneration, expected] {
            const QDBusPendingReply<uint> reply = *watcher;
            watcher->deleteLater();
            if (requestGeneration != generation)
                return;
            pending = false;
            if (reply.isError() || resolve() != std::optional{expected}) {
                revoke();
                return;
            }
            annotation = xdg_dbus_annotation_manager_v1_annotate_surface(
                manager->proxy, interfaceName, expected.surface);
            xdg_dbus_annotation_v1_set_bus(annotation, XDG_DBUS_ANNOTATION_V1_BUS_ENUM_ATSPI);
            xdg_dbus_annotation_v1_set_bus_name(annotation, expected.owner.toUtf8().constData());
            xdg_dbus_annotation_v1_set_object_path(annotation, expected.path.toUtf8().constData());
            nativeWindow->commit();
        });
    }

private:
    std::optional<Endpoint> resolve()
    {
        if (!manager->isActive() || !manager->proxy || !window || !window->isVisible()
            || !window->handle() || !QAccessible::isActive())
            return std::nullopt;
        auto connection = bridgeConnection();
        if (!connection)
            return std::nullopt;
        const auto owner = connection->baseService();
        if (!connection->isConnected() || !owner.startsWith(QLatin1Char(':')))
            return std::nullopt;
        auto *native = dynamic_cast<QWaylandWindow *>(window->handle());
        if (!native)
            return std::nullopt;
        if (nativeWindow != native) {
            if (nativeWindow)
                disconnect(nativeWindow, nullptr, this, nullptr);
            nativeWindow = native;
            connect(native, &QWaylandWindow::wlSurfaceDestroyed, this, &Binding::revoke,
                    Qt::DirectConnection);
            connect(native, &QWaylandWindow::wlSurfaceCreated, this, [this] {
                QTimer::singleShot(0, this, &Binding::refresh);
            });
            connect(native, &QObject::destroyed, this, &Binding::revoke, Qt::DirectConnection);
        }
        auto *surface = static_cast<wl_surface *>(QGuiApplication::platformNativeInterface()
                            ->nativeResourceForWindow("surface", window));
        auto *root = window->accessibleRoot();
        if (!surface || !root || !root->isValid() || root->role() == QAccessible::Application)
            return std::nullopt;
        // Qt 6.12.0 AtSpiAdaptor::pathForInterface, also reused by Qt change 717474.
        const auto id = QAccessible::uniqueId(root);
        if (static_cast<int>(id) >= 0)
            return std::nullopt;
        return Endpoint{owner, QStringLiteral("/org/a11y/atspi/accessible/") + QString::number(id), surface};
    }

    QPointer<QWindow> window;
    QPointer<QWaylandWindow> nativeWindow;
    Manager *manager;
    std::optional<Endpoint> endpoint;
    xdg_dbus_annotation_v1 *annotation = nullptr;
    quint64 generation = 0;
    bool pending = false;
};

class Producer final : public QObject, public QAccessible::ActivationObserver
{
public:
    explicit Producer(QObject *parent) : QObject(parent)
    {
        QTimer::singleShot(0, this, [this] {
            if (QString::fromLatin1(qVersion()) != QString::fromUtf8(DOJO_QT_VERSION)
                || runtimeLibrariesPath() != QString::fromUtf8(DOJO_QT_LIBRARY_PATH)
                || (QGuiApplication::platformName() != QStringLiteral("wayland")
                    && QGuiApplication::platformName() != QStringLiteral("wayland-egl"))) {
                qWarning("Dojo annotation plugin requires its pinned Qt Wayland ABI");
                return;
            }
            manager = new Manager;
            manager->setParent(this);
            qGuiApp->setProperty("dojoQt6AnnotationLoaded", true);
            connect(manager, &QWaylandClientExtension::activeChanged, this, [this] {
                qGuiApp->setProperty("dojoQt6AnnotationGlobalActive", manager->isActive());
                refresh();
            });
            QAccessible::installActivationObserver(this);
            observing = true;
            qGuiApp->installEventFilter(this);
            timer.setInterval(250);
            connect(&timer, &QTimer::timeout, this, [this] { refresh(); });
            timer.start();
            refresh();
        });
    }
    ~Producer() override
    {
        if (observing)
            QAccessible::removeActivationObserver(this);
        qDeleteAll(bindings);
        bindings.clear();
        delete manager;
    }
    void accessibilityActiveChanged(bool active) override
    {
        if (!active)
            for (auto *binding : std::as_const(bindings))
                binding->revoke();
        QTimer::singleShot(0, this, [this] { refresh(); });
    }
    bool eventFilter(QObject *, QEvent *event) override
    {
        if (event->type() == QEvent::Show || event->type() == QEvent::Hide
            || event->type() == QEvent::PlatformSurface)
            QTimer::singleShot(0, this, [this] { refresh(); });
        return false;
    }
private:
    void refresh()
    {
        if (!manager)
            return;
        const auto connection = bridgeConnection();
        qGuiApp->setProperty("dojoQtAnnotationBridgeAvailable", connection && connection->isConnected()
                            && connection->baseService().startsWith(QLatin1Char(':')));
        for (auto *window : QGuiApplication::topLevelWindows()) {
            if (!bindings.contains(window)) {
                auto *binding = new Binding(window, manager, this);
                bindings.insert(window, binding);
                connect(window, &QObject::destroyed, this, [this, window] {
                    delete bindings.take(window);
                });
            }
        }
        for (auto *binding : std::as_const(bindings))
            binding->refresh();
    }
    Manager *manager = nullptr;
    QHash<QWindow *, Binding *> bindings;
    QTimer timer;
    bool observing = false;
};
} // namespace

class AnnotationPlugin final : public QGenericPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QGenericPluginFactoryInterface_iid FILE "plugin.json")
public:
    QObject *create(const QString &key, const QString &) override
    {
        if (key != QString::fromUtf8(DOJO_ANNOTATION_KEY)) {
            if (key == QStringLiteral("dojo-qt5-annotation-5.15.19")
                || key == QStringLiteral("dojo-qt6-annotation-6.12.0"))
                return new QObject;
            return nullptr;
        }
        static QPointer<Producer> instance;
        if (instance)
            return new QObject;
        instance = new Producer(nullptr);
        return instance;
    }
};

#include "annotation.moc"
