cmake_minimum_required(VERSION 3.20)
project(envbox-runtime LANGUAGES CXX)

# Microsoft Detours (ticket 04). DETOURS_ROOT must contain include/ and lib.X64/
# (or lib.X86/). Build Detours first: vcvars + nmake in Detours/src.
option(ENVBOX_WITH_DETOURS "Link Microsoft Detours" ON)
set(DETOURS_ROOT "" CACHE PATH "Path to Microsoft Detours source/install")

if(CMAKE_SIZEOF_VOID_P EQUAL 8)
  set(ENVBOX_RT_TARGET envbox-runtime64)
  set(ENVBOX_RT_NAME envbox-runtime64)
  set(ENVBOX_RT_ARCH 64)
  set(ENVBOX_RT_EXPORTS src/envbox-runtime64.def)
else()
  set(ENVBOX_RT_TARGET envbox-runtime32)
  set(ENVBOX_RT_NAME envbox-runtime32)
  set(ENVBOX_RT_ARCH 32)
  set(ENVBOX_RT_EXPORTS src/envbox-runtime32.def)
endif()

if(ENVBOX_WITH_DETOURS AND NOT DETOURS_ROOT)
  if(EXISTS "D:/Tools/Detours/include/detours.h")
    set(DETOURS_ROOT "D:/Tools/Detours")
  elseif(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../third_party/Detours/include/detours.h")
    set(DETOURS_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../third_party/Detours")
  endif()
endif()

add_library(${ENVBOX_RT_TARGET} SHARED
  ${ENVBOX_RT_EXPORTS}
  src/dllmain.cpp
  src/audit.cpp
  src/runtime_profile.cpp
  src/ipc_bootstrap.cpp
  src/service_bootstrap.cpp
  src/startup_gate.cpp
  src/hooks_time.cpp
  src/hooks_winrt_time.cpp
  src/hooks_geo.cpp
  src/hooks_locale.cpp
  src/hooks_crt_locale.cpp
  src/hooks_language.cpp
  src/hooks_dns.cpp
  src/dns_transport.cpp
  src/dns_dot.cpp
  src/dns_doh.cpp
  src/dns_profile_config.cpp
  src/hooks_registry.cpp
  src/browser_child_locale.cpp
  src/hooks_identity.cpp
  src/identity_profile_config.cpp
  src/hooks_process.cpp
  src/hooks_network.cpp
)
target_compile_features(${ENVBOX_RT_TARGET} PRIVATE cxx_std_17)
target_compile_definitions(${ENVBOX_RT_TARGET} PRIVATE ENVBOX_RUNTIME=1 ENVBOX_ARCH=${ENVBOX_RT_ARCH})
file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/../Cargo.toml" ENVBOX_VERSION_LINE REGEX "^version = \"[0-9]+\\.[0-9]+\\.[0-9]+[^\"]*\"$")
list(GET ENVBOX_VERSION_LINE 0 ENVBOX_VERSION_LINE)
string(REGEX REPLACE "^version = \"([^\"]+)\"$" "\\1" ENVBOX_RUNTIME_VERSION "${ENVBOX_VERSION_LINE}")
target_compile_definitions(${ENVBOX_RT_TARGET} PRIVATE ENVBOX_RUNTIME_VERSION="${ENVBOX_RUNTIME_VERSION}")
set_target_properties(${ENVBOX_RT_TARGET} PROPERTIES OUTPUT_NAME "${ENVBOX_RT_NAME}")
# Rust's MSVC staticlib uses the dynamic release CRT in both Cargo profiles.
# Match it for C++ Debug too; retain debug symbols and disabled optimization.
set_property(TARGET ${ENVBOX_RT_TARGET} PROPERTY MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE advapi32 iphlpapi ws2_32 runtimeobject secur32 crypt32 shell32)

# DoH is part of the shipped Runtime, not a separately enabled prototype. Each
# CMake build tree owns its Rust target directory, avoiding concurrent installer
# builds and loaded Runtime artifacts in the workspace target directory.
if(NOT MSVC)
  message(FATAL_ERROR "Runtime DoH integration requires MSVC")
endif()
find_program(ENVBOX_POWERSHELL NAMES powershell REQUIRED)
if(ENVBOX_RT_ARCH EQUAL 64)
  set(ENVBOX_DOH_RUST_TARGET x86_64-pc-windows-msvc)
else()
  set(ENVBOX_DOH_RUST_TARGET i686-pc-windows-msvc)
endif()
execute_process(COMMAND "${ENVBOX_POWERSHELL}" -NoProfile -NonInteractive
  -ExecutionPolicy Bypass -File "${CMAKE_CURRENT_SOURCE_DIR}/build-doh-staticlib.ps1"
  -BuildDirectory "${CMAKE_CURRENT_BINARY_DIR}" -RustTarget "${ENVBOX_DOH_RUST_TARGET}"
  -Configuration Debug RESULT_VARIABLE ENVBOX_DOH_BUILD_STATUS)
if(NOT ENVBOX_DOH_BUILD_STATUS EQUAL 0)
  message(FATAL_ERROR "DoH staticlib configuration build failed")
endif()
include("${CMAKE_CURRENT_BINARY_DIR}/doh-native-libs.cmake")
add_custom_target(envbox-doh-staticlib
  COMMAND "${ENVBOX_POWERSHELL}" -NoProfile -NonInteractive -ExecutionPolicy Bypass
    -File "${CMAKE_CURRENT_SOURCE_DIR}/build-doh-staticlib.ps1"
    -BuildDirectory "${CMAKE_CURRENT_BINARY_DIR}" -RustTarget "${ENVBOX_DOH_RUST_TARGET}"
    -Configuration "$<CONFIG>"
  BYPRODUCTS "${CMAKE_CURRENT_BINARY_DIR}/doh-product/${ENVBOX_DOH_RUST_TARGET}/debug/envbox_dns_doh.lib"
             "${CMAKE_CURRENT_BINARY_DIR}/doh-product/${ENVBOX_DOH_RUST_TARGET}/release/envbox_dns_doh.lib"
  VERBATIM)
add_dependencies(${ENVBOX_RT_TARGET} envbox-doh-staticlib)
target_include_directories(${ENVBOX_RT_TARGET} PRIVATE ../crates/envbox-dns-doh/include)
target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE
  "$<IF:$<CONFIG:Debug>,${CMAKE_CURRENT_BINARY_DIR}/doh-product/${ENVBOX_DOH_RUST_TARGET}/debug/envbox_dns_doh.lib,${CMAKE_CURRENT_BINARY_DIR}/doh-product/${ENVBOX_DOH_RUST_TARGET}/release/envbox_dns_doh.lib>"
  ${ENVBOX_DOH_NATIVE_LIBS})
target_link_options(${ENVBOX_RT_TARGET} PRIVATE ${ENVBOX_DOH_NATIVE_OPTIONS})

if(ENVBOX_WITH_DETOURS)
  if(NOT DETOURS_ROOT)
    message(FATAL_ERROR "ENVBOX_WITH_DETOURS requires DETOURS_ROOT (or D:/Tools/Detours)")
  endif()
  target_include_directories(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/include")
  if(ENVBOX_RT_ARCH EQUAL 64)
    target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/lib.X64/detours.lib")
  else()
    target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/lib.X86/detours.lib")
  endif()
  message(STATUS "envbox-runtime: linking Detours from ${DETOURS_ROOT}")
else()
  message(WARNING "envbox-runtime: building WITHOUT Detours (ticket 04 requires Detours)")
endif()
