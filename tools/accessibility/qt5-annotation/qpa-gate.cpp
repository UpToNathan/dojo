// SPDX-License-Identifier: LGPL-3.0-only
#include <QCoreApplication>
#include <QGuiApplication>
#include <QLibraryInfo>
#include <QPluginLoader>
#include <QVariant>
#include <QtGui/qpa/qplatformintegrationplugin.h>

class PinnedPlatformGate final : public QPlatformIntegrationPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QPlatformIntegrationFactoryInterface_iid FILE "qpa-gate.json")
public:
    QPlatformIntegration *create(const QString &key, const QStringList &parameters,
                                 int &argc, char **argv) override
    {
        if (QString::fromLatin1(qVersion()) != QStringLiteral("5.15.19")
            || QLibraryInfo::location(QLibraryInfo::LibrariesPath)
                != QString::fromUtf8(DOJO_QT_LIBRARY_PATH))
            return nullptr;
        QString file;
        if (key == QStringLiteral("wayland"))
            file = QStringLiteral("libqwayland-generic.so");
        else if (key == QStringLiteral("wayland-egl"))
            file = QStringLiteral("libqwayland-egl.so");
        else if (key == QStringLiteral("wayland-xcomposite-egl"))
            file = QStringLiteral("libqwayland-xcomposite-egl.so");
        else if (key == QStringLiteral("wayland-xcomposite-glx"))
            file = QStringLiteral("libqwayland-xcomposite-glx.so");
        else
            return nullptr;
        const auto root = QString::fromUtf8(DOJO_QT5_BACKEND_PLUGINS);
        QCoreApplication::addLibraryPath(root);
        auto *loader = new QPluginLoader(root + QStringLiteral("/platforms/") + file, qGuiApp);
        loader->setLoadHints(loader->loadHints() | QLibrary::PreventUnloadHint);
        auto *factory = qobject_cast<QPlatformIntegrationPlugin *>(loader->instance());
        if (!factory) {
            delete loader;
            return nullptr;
        }
        qGuiApp->setProperty("dojoQt5QpaGateAccepted", true);
        return factory->create(key, parameters, argc, argv);
    }
};

#include "qpa-gate.moc"
