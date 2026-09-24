cmake_minimum_required(VERSION 3.20)
project(envbox-runtime LANGUAGES CXX)

# Microsoft Detours is integrated in ticket 04. For ticket 01 this builds
# empty runtime DLLs so the launcher injection seam can be wired later.
option(ENVBOX_WITH_DETOURS "Link Microsoft Detours" OFF)
set(DETOURS_ROOT "" CACHE PATH "Path to Microsoft Detours source/install")

if(CMAKE_SIZEOF_VOID_P EQUAL 8)
  set(ENVBOX_RT_TARGET envbox-runtime64)
  set(ENVBOX_RT_NAME envbox-runtime64)
  set(ENVBOX_RT_ARCH 64)
else()
  set(ENVBOX_RT_TARGET envbox-runtime32)
  set(ENVBOX_RT_NAME envbox-runtime32)
  set(ENVBOX_RT_ARCH 32)
endif()

add_library(${ENVBOX_RT_TARGET} SHARED src/dllmain.cpp)
target_compile_features(${ENVBOX_RT_TARGET} PRIVATE cxx_std_17)
target_compile_definitions(${ENVBOX_RT_TARGET} PRIVATE ENVBOX_RUNTIME=1 ENVBOX_ARCH=${ENVBOX_RT_ARCH})
set_target_properties(${ENVBOX_RT_TARGET} PROPERTIES OUTPUT_NAME "${ENVBOX_RT_NAME}")

if(ENVBOX_WITH_DETOURS)
  if(NOT DETOURS_ROOT)
    message(FATAL_ERROR "ENVBOX_WITH_DETOURS requires DETOURS_ROOT")
  endif()
  target_include_directories(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/include")
  if(ENVBOX_RT_ARCH EQUAL 64)
    target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/lib.X64/detours.lib")
  else()
    target_link_libraries(${ENVBOX_RT_TARGET} PRIVATE "${DETOURS_ROOT}/lib.X86/detours.lib")
  endif()
endif()
