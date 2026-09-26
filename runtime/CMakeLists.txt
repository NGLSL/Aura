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
  src/hooks_time.cpp
  src/hooks_geo.cpp
  src/hooks_locale.cpp
  src/hooks_language.cpp
  src/hooks_dns.cpp
  src/hooks_registry.cpp
  src/hooks_process.cpp
  src/hooks_network.cpp
)
target_compile_features(${ENVBOX_RT_TARGET} PRIVATE cxx_std_17)
target_compile_definitions(${ENVBOX_RT_TARGET} PRIVATE ENVBOX_RUNTIME=1 ENVBOX_ARCH=${ENVBOX_RT_ARCH})
set_target_properties(${ENVBOX_RT_TARGET} PROPERTIES OUTPUT_NAME "${ENVBOX_RT_NAME}")
target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE iphlpapi ws2_32)

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
