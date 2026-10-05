cmake_minimum_required(VERSION 3.20)
project(envbox-dns-doh-native-probe LANGUAGES CXX)

if(NOT MSVC OR NOT IS_ABSOLUTE "${DOH_STATICLIB}")
  message(FATAL_ERROR "MSVC and absolute DOH_STATICLIB are required")
endif()

add_library(doh-native-probe SHARED bridge.cpp probe.def)
target_include_directories(doh-native-probe PRIVATE ../../crates/envbox-dns-doh/include)
target_link_libraries(doh-native-probe PRIVATE "${DOH_STATICLIB}" ${DOH_NATIVE_LIBS})
target_link_options(doh-native-probe PRIVATE ${DOH_NATIVE_OPTIONS})

add_executable(doh-native-probe-host host.cpp)
target_link_libraries(doh-native-probe-host PRIVATE ws2_32 iphlpapi psapi)
set_property(TARGET doh-native-probe doh-native-probe-host PROPERTY MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
