cmake_minimum_required(VERSION 3.24)
project(dojo_qt5_annotation VERSION 1.0 LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_AUTOMOC ON)
find_package(Qt5 5.15.19 EXACT REQUIRED COMPONENTS Core Gui DBus WaylandClient Widgets XkbCommonSupport)
find_package(PkgConfig REQUIRED)
pkg_check_modules(Wayland REQUIRED IMPORTED_TARGET wayland-client)
find_program(WAYLAND_SCANNER wayland-scanner REQUIRED)
set(PROTOCOL ${CMAKE_CURRENT_SOURCE_DIR}/../qt6-annotation/xdg-dbus-annotation-v1.xml)
file(SHA256 ${PROTOCOL} PROTOCOL_HASH)
if(NOT PROTOCOL_HASH STREQUAL "e5ee1c96ccd28423c7ffd4429e24db132552770f9d15a831c46560b3636ee0bb")
    message(FATAL_ERROR "MR493 protocol differs from the pinned provenance")
endif()
configure_file(../qt6-annotation/annotation.cpp annotation.cpp COPYONLY)
configure_file(../qt6-annotation/fixture.cpp fixture.cpp COPYONLY)
configure_file(plugin.json plugin.json COPYONLY)
add_custom_command(OUTPUT annotation-client.h
    COMMAND ${WAYLAND_SCANNER} client-header ${PROTOCOL} annotation-client.h DEPENDS ${PROTOCOL})
add_custom_command(OUTPUT annotation-protocol.c
    COMMAND ${WAYLAND_SCANNER} private-code ${PROTOCOL} annotation-protocol.c DEPENDS ${PROTOCOL})
add_library(dojo_qt5_annotation MODULE ${CMAKE_CURRENT_BINARY_DIR}/annotation.cpp
    annotation-client.h annotation-protocol.c)
target_include_directories(dojo_qt5_annotation PRIVATE ${CMAKE_CURRENT_BINARY_DIR})
target_link_libraries(dojo_qt5_annotation PRIVATE Qt5::GuiPrivate Qt5::DBus
    Qt5::WaylandClientPrivate Qt5::XkbCommonSupportPrivate PkgConfig::Wayland ${CMAKE_DL_LIBS})
get_target_property(QT_CORE_LIBRARY Qt5::Core IMPORTED_LOCATION_RELEASE)
get_filename_component(QT_LIBRARY_PATH "${QT_CORE_LIBRARY}" DIRECTORY)
target_compile_definitions(dojo_qt5_annotation PRIVATE DOJO_QT_LIBRARY_PATH="${QT_LIBRARY_PATH}"
    DOJO_QT_VERSION="5.15.19" DOJO_ANNOTATION_KEY="dojo-qt5-annotation-5.15.19")
if(NOT DOJO_QT5_BACKEND_PLUGINS)
    message(FATAL_ERROR "DOJO_QT5_BACKEND_PLUGINS must identify the pinned patched backend's plugin directory")
endif()
add_library(dojo_qt5_qpa_gate MODULE qpa-gate.cpp qpa-gate.json)
target_link_libraries(dojo_qt5_qpa_gate PRIVATE Qt5::GuiPrivate)
target_compile_definitions(dojo_qt5_qpa_gate PRIVATE DOJO_QT_LIBRARY_PATH="${QT_LIBRARY_PATH}"
    DOJO_QT5_BACKEND_PLUGINS="${DOJO_QT5_BACKEND_PLUGINS}")
set_target_properties(dojo_qt5_annotation PROPERTIES LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/plugins/generic)
add_executable(dojo_qt5_annotation_fixture ${CMAKE_CURRENT_BINARY_DIR}/fixture.cpp)
target_link_libraries(dojo_qt5_annotation_fixture PRIVATE Qt5::Widgets)
install(TARGETS dojo_qt5_annotation LIBRARY DESTINATION lib/dojo/qt-5.15.19/plugins/generic)
install(TARGETS dojo_qt5_qpa_gate LIBRARY DESTINATION lib/dojo/qt-5.15.19/qpa-gate)
install(TARGETS dojo_qt5_annotation_fixture RUNTIME DESTINATION libexec)
install(FILES provenance.json README.md qtwayland-atspi-connection.patch
    DESTINATION share/doc/dojo-qt5-annotation)
install(FILES ../qt6-annotation/provenance.json
    DESTINATION share/doc/dojo-qt5-annotation/shared-qt6)
install(DIRECTORY licenses/ DESTINATION share/doc/dojo-qt5-annotation/licenses/qt5)
install(DIRECTORY ../qt6-annotation/licenses/
    DESTINATION share/doc/dojo-qt5-annotation/licenses/shared-qt6)
