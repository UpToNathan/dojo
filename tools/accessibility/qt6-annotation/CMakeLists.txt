cmake_minimum_required(VERSION 3.24)
project(dojo_qt6_annotation VERSION 1.0 LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_AUTOMOC ON)
find_package(Qt6 6.12.0 EXACT REQUIRED COMPONENTS Core Gui GuiPrivate DBus WaylandClient WaylandClientPrivate Widgets)
find_package(PkgConfig REQUIRED)
pkg_check_modules(Wayland REQUIRED IMPORTED_TARGET wayland-client)
find_program(WAYLAND_SCANNER wayland-scanner REQUIRED)
set(PROTOCOL ${CMAKE_CURRENT_SOURCE_DIR}/xdg-dbus-annotation-v1.xml)
file(SHA256 ${PROTOCOL} PROTOCOL_HASH)
if(NOT PROTOCOL_HASH STREQUAL "e5ee1c96ccd28423c7ffd4429e24db132552770f9d15a831c46560b3636ee0bb")
    message(FATAL_ERROR "MR493 protocol differs from the pinned provenance")
endif()
add_custom_command(OUTPUT annotation-client.h
    COMMAND ${WAYLAND_SCANNER} client-header ${PROTOCOL} annotation-client.h DEPENDS ${PROTOCOL})
add_custom_command(OUTPUT annotation-protocol.c
    COMMAND ${WAYLAND_SCANNER} private-code ${PROTOCOL} annotation-protocol.c DEPENDS ${PROTOCOL})
add_library(dojo_qt6_annotation MODULE
    annotation.cpp plugin.json annotation-client.h annotation-protocol.c)
target_include_directories(dojo_qt6_annotation PRIVATE ${CMAKE_CURRENT_BINARY_DIR})
target_link_libraries(dojo_qt6_annotation PRIVATE Qt6::GuiPrivate Qt6::DBus
    Qt6::WaylandClientPrivate PkgConfig::Wayland)
get_target_property(QT_CORE_LIBRARY Qt6::Core IMPORTED_LOCATION_RELEASE)
get_filename_component(QT_LIBRARY_PATH "${QT_CORE_LIBRARY}" DIRECTORY)
target_compile_definitions(dojo_qt6_annotation PRIVATE DOJO_QT_LIBRARY_PATH="${QT_LIBRARY_PATH}"
    DOJO_QT_VERSION="6.12.0" DOJO_ANNOTATION_KEY="dojo-qt6-annotation-6.12.0")
set_target_properties(dojo_qt6_annotation PROPERTIES LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/plugins/generic)
qt_add_executable(dojo_qt6_annotation_fixture fixture.cpp)
target_link_libraries(dojo_qt6_annotation_fixture PRIVATE Qt6::Widgets)
install(TARGETS dojo_qt6_annotation LIBRARY DESTINATION lib/dojo/qt-6.12.0/plugins/generic)
install(TARGETS dojo_qt6_annotation_fixture RUNTIME DESTINATION libexec)
install(FILES provenance.json README.md xdg-dbus-annotation-v1.xml
    DESTINATION share/doc/dojo-qt6-annotation)
install(DIRECTORY licenses/ DESTINATION share/doc/dojo-qt6-annotation/licenses)
