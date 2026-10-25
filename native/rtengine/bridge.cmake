# Included by the pinned upstream project after its rtengine target exists.
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set_target_properties(rtengine PROPERTIES POSITION_INDEPENDENT_CODE ON)
configure_file("${PROJECT_SOURCE_DIR}/rtgui/config.h.in" "${CMAKE_BINARY_DIR}/rtgui/config.h")
add_library(openroom_rtengine SHARED
    "${OPENROOM_BRIDGE_DIR}/bridge.cc"
    rtgui/alignedmalloc.cc
    rtgui/editcallbacks.cc
    rtgui/multilangmgr.cc
    rtgui/options.cc
    rtgui/paramsedited.cc
    rtgui/pathutils.cc
    rtgui/threadutils.cc)
target_include_directories(openroom_rtengine PRIVATE
    "${PROJECT_SOURCE_DIR}/rtgui" "${CMAKE_BINARY_DIR}/rtgui")
target_link_libraries(openroom_rtengine PRIVATE rtengine)
target_link_options(openroom_rtengine PRIVATE "-Wl,-dead_strip_dylibs")
target_compile_definitions(openroom_rtengine PRIVATE CLIVERSION)
set_target_properties(openroom_rtengine PROPERTIES
    CXX_STANDARD 11 INSTALL_NAME_DIR "${CMAKE_BINARY_DIR}"
    BUILD_WITH_INSTALL_NAME_DIR ON)
include("${PROJECT_SOURCE_DIR}/cmake/AboutThisBuild.cmake")
rt_generate_about_command(OPENROOM_ABOUT_COMMAND rtengine openroom_rtengine)
add_custom_target(UpdateInfo COMMAND ${OPENROOM_ABOUT_COMMAND})
add_dependencies(openroom_rtengine UpdateInfo)
