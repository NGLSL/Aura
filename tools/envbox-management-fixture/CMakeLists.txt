cmake_minimum_required(VERSION 3.20)
project(envbox-management-fixture LANGUAGES CXX)

add_executable(envbox-management-fixture main.cpp)
target_compile_features(envbox-management-fixture PRIVATE cxx_std_17)
target_compile_definitions(envbox-management-fixture PRIVATE UNICODE _UNICODE)
target_compile_options(envbox-management-fixture PRIVATE /W4 /EHsc)
target_link_libraries(envbox-management-fixture PRIVATE advapi32)
