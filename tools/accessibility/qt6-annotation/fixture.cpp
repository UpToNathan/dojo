#include <QApplication>
#include <QAccessible>
#include <QLineEdit>
#include <QPushButton>
#include <QTimer>
#include <QVBoxLayout>
#include <QWidget>
#include <QWindow>
#include <cstdio>

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    app.setApplicationName("dojo-qt6-annotation-fixture");
    QWidget first;
    QWidget second;
    for (auto [window, name] : {std::pair{&first, "First fixture window"},
                                std::pair{&second, "Second fixture window"}}) {
        window->setWindowTitle(QString::fromLatin1(name));
        window->setAccessibleName(QString::fromLatin1(name));
        auto *layout = new QVBoxLayout(window);
        auto *edit = new QLineEdit(window);
        edit->setAccessibleName("Fixture input");
        layout->addWidget(edit);
        auto *recreate = new QPushButton("Recreate this native surface", window);
        layout->addWidget(recreate);
        QObject::connect(recreate, &QPushButton::clicked, window, [window] {
            window->hide();
            window->windowHandle()->destroy();
            QTimer::singleShot(100, window, [window] { window->show(); });
        });
        window->resize(400, 180);
        window->show();
    }
    if (app.arguments().contains("--exercise-lifecycle")) {
        QTimer::singleShot(300, &app, [&] {
            auto *firstRoot = first.windowHandle()->accessibleRoot();
            auto *secondRoot = second.windowHandle()->accessibleRoot();
            if (!firstRoot || !secondRoot
                || QAccessible::uniqueId(firstRoot) == QAccessible::uniqueId(secondRoot))
                qFatal("Fixture roots must be distinct within one application");
            const auto plugins = qgetenv("QT_QPA_GENERIC_PLUGINS");
            const bool pluginExpected = plugins.contains("dojo-qt6-annotation-6.12.0")
                || plugins.contains("dojo-qt5-annotation-5.15.19");
            const bool loaded = app.property("dojoQt6AnnotationLoaded").toBool();
            const bool expectRefusal = app.arguments().contains("--expect-plugin-refusal");
            if ((pluginExpected && !loaded && !expectRefusal) || (expectRefusal && loaded))
                qFatal("Fixture expected the Qt annotation plugin to load");
            std::fprintf(stderr, "distinct_roots=1 plugin_loaded=%d annotation_global_active=%d bridge_available=%d qpa_gate_accepted=%d\n",
                         loaded, app.property("dojoQt6AnnotationGlobalActive").toBool(),
                         app.property("dojoQtAnnotationBridgeAvailable").toBool(),
                         app.property("dojoQt5QpaGateAccepted").toBool());
            first.hide();
            first.windowHandle()->destroy();
            QTimer::singleShot(100, &first, [&] { first.show(); });
        });
        QTimer::singleShot(800, &app, [] { QAccessible::setActive(false); });
        QTimer::singleShot(1000, &app, [] { QAccessible::setActive(true); });
        QTimer::singleShot(1800, &app, &QApplication::quit);
    }
    return app.exec();
}
