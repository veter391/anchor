@echo off
REM Dev/build environment for Anchor once the embedded llama.cpp engine (llama-cpp-2)
REM is a dependency: it needs MSVC (cl.exe), CMake, Ninja and libclang on PATH, and
REM ggml built WITHOUT OpenMP so it doesn't clash with our ONNX Runtime stack.
REM Usage:  build-env.bat            -> opens a shell with the env set
REM         build-env.bat <cmd...>   -> runs <cmd> in that env (e.g. pnpm tauri dev)
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set "PATH=D:\dev\cargo\bin;D:\dev\LLVM\bin;C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%"
set "CARGO_HOME=D:\dev\cargo"
set "RUSTUP_HOME=D:\dev\rustup"
set "TMP=D:\dev\tmp"
set "TEMP=D:\dev\tmp"
set "LIBCLANG_PATH=D:\dev\LLVM\bin"
set "CMAKE_GENERATOR=Ninja"
REM Disable ggml OpenMP to coexist with onnxruntime's OpenMP in one process.
set "CMAKE_ARGS=-DGGML_OPENMP=OFF"
cd /d D:\Projects\Anchor
if "%~1"=="" (
  cmd /k
) else (
  %*
)
