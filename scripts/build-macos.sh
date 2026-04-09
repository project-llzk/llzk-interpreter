#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

# Homebrew LLVM default locations (Apple Silicon first, then Intel).
if [[ -z "${LLVM_PREFIX:-}" ]]; then
  if [[ -d "/opt/homebrew/opt/llvm@20" ]]; then
    LLVM_PREFIX="/opt/homebrew/opt/llvm@20"
  elif [[ -d "/usr/local/opt/llvm@20" ]]; then
    LLVM_PREFIX="/usr/local/opt/llvm@20"
  else
    echo "error: llvm@20 not found. Set LLVM_PREFIX manually." >&2
    exit 1
  fi
fi

if [[ -z "${LLZK_PCL_ROOT:-}" ]]; then
  default_pcl_root="${repo_root}/../pcl-mlir"
  if [[ -d "${default_pcl_root}" ]]; then
    LLZK_PCL_ROOT="${default_pcl_root}"
  else
    echo "error: LLZK_PCL_ROOT is not set and ${default_pcl_root} does not exist." >&2
    echo "set LLZK_PCL_ROOT to your local pcl-mlir checkout." >&2
    exit 1
  fi
fi

if [[ -z "${LLZK_PCL_PREFIX:-}" ]]; then
  LLZK_PCL_PREFIX="${LLZK_PCL_ROOT}/build"
fi

if [[ ! -d "${LLZK_PCL_PREFIX}" ]]; then
  echo "error: LLZK_PCL_PREFIX does not exist: ${LLZK_PCL_PREFIX}" >&2
  echo "build pcl-mlir first or set LLZK_PCL_PREFIX to the correct build directory." >&2
  exit 1
fi

if [[ -z "${LLZK_LIB_ROOT:-}" ]]; then
  default_llzk_root="${repo_root}/../llzk-lib"
  if [[ -d "${default_llzk_root}" ]]; then
    LLZK_LIB_ROOT="${default_llzk_root}"
  fi
fi

export MLIR_SYS_200_PREFIX="${MLIR_SYS_200_PREFIX:-${LLVM_PREFIX}}"
export TABLEGEN_200_PREFIX="${TABLEGEN_200_PREFIX:-${LLVM_PREFIX}}"
export LIBCLANG_PATH="${LIBCLANG_PATH:-${LLVM_PREFIX}/lib}"
export LLZK_PCL_ROOT
export LLZK_PCL_PREFIX
if [[ -n "${LLZK_LIB_ROOT:-}" ]]; then
  export LLZK_SYS_10_PREFIX="${LLZK_SYS_10_PREFIX:-${LLZK_LIB_ROOT}/build/install}"
fi
export SDKROOT="${SDKROOT:-$(xcrun --show-sdk-path)}"

# Keep any existing RUSTFLAGS while guaranteeing Homebrew lib lookup for zstd.
if [[ "${RUSTFLAGS:-}" == *"-L /opt/homebrew/lib/"* || "${RUSTFLAGS:-}" == *"-L/opt/homebrew/lib/"* ]]; then
  export RUSTFLAGS
else
  export RUSTFLAGS="${RUSTFLAGS:-} -L /opt/homebrew/lib/"
fi

echo "Building with:"
echo "  MLIR_SYS_200_PREFIX=${MLIR_SYS_200_PREFIX}"
echo "  TABLEGEN_200_PREFIX=${TABLEGEN_200_PREFIX}"
echo "  LIBCLANG_PATH=${LIBCLANG_PATH}"
echo "  LLZK_PCL_ROOT=${LLZK_PCL_ROOT}"
echo "  LLZK_PCL_PREFIX=${LLZK_PCL_PREFIX}"
echo "  SDKROOT=${SDKROOT}"

cd "${repo_root}"
if [[ "$#" -eq 0 ]]; then
  cargo build
else
  cargo "$@"
fi
