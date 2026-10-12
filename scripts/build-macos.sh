#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
# The crate-specific prefix also wins over an inherited generic LLVM_PREFIX
# when entering a Nix shell from a manually configured environment.
LLVM_PREFIX="${MLIR_SYS_230_PREFIX:-${LLVM_PREFIX:-}}"

if [[ -z "${LLVM_PREFIX:-}" ]]; then
  echo "error: set LLVM_PREFIX or MLIR_SYS_230_PREFIX to the LLVM/MLIR installation used to build LLZK." >&2
  echo "alternatively, run this command inside 'nix develop'." >&2
  exit 1
fi

export MLIR_SYS_230_PREFIX="${MLIR_SYS_230_PREFIX:-${LLVM_PREFIX}}"
export TABLEGEN_230_PREFIX="${TABLEGEN_230_PREFIX:-${LLVM_PREFIX}}"
for prefix in "${MLIR_SYS_230_PREFIX}" "${TABLEGEN_230_PREFIX}"; do
  if [[ ! -x "${prefix}/bin/llvm-config" ]] || \
     [[ "$("${prefix}/bin/llvm-config" --version)" != 23.* ]]; then
    echo "error: ${prefix}/bin/llvm-config must report LLVM 23.x." >&2
    exit 1
  fi
done
export LIBCLANG_PATH="${LIBCLANG_PATH:-${LLVM_PREFIX}/lib}"
export LLZK_SYS_30_PREFIX="${LLZK_SYS_30_PREFIX:-${LLZK_LIB_ROOT:-${repo_root}/../llzk-lib}/build/install}"
if [[ ! -d "${LLZK_SYS_30_PREFIX}/include/llzk" || \
      ! -f "${LLZK_SYS_30_PREFIX}/lib/libLLZKCAPI.a" ]]; then
  echo "error: LLZK 3 installation not found at ${LLZK_SYS_30_PREFIX}." >&2
  echo "set LLZK_SYS_30_PREFIX to LLZK's install directory (built with LLVM 23)." >&2
  exit 1
fi

# Installed LLZK exports the LLVM headers it was built with. Check the build
# configuration, since matching LLVM version numbers alone do not imply ABI
# compatibility (notably for Nix debug versus Homebrew release builds).
llzk_targets="${LLZK_SYS_30_PREFIX}/lib/cmake/LLZK/LLZKTargets.cmake"
if [[ -f "${llzk_targets}" ]]; then
  llzk_llvm_include="$(awk '
    /^set_target_properties\(LLZK::LLVMHeaders PROPERTIES/ { in_target = 1 }
    in_target && /INTERFACE_INCLUDE_DIRECTORIES/ { split($0, parts, "\""); print parts[2]; exit }
  ' "${llzk_targets}")"
  for prefix in "${MLIR_SYS_230_PREFIX}" "${TABLEGEN_230_PREFIX}"; do
    llvm_include="$("${prefix}/bin/llvm-config" --includedir)"
    for config in abi-breaking.h llvm-config.h; do
      if [[ -f "${llzk_llvm_include}/llvm/Config/${config}" ]] && \
         ! cmp -s "${llzk_llvm_include}/llvm/Config/${config}" "${llvm_include}/llvm/Config/${config}"; then
        echo "error: LLZK and ${prefix} use different LLVM build configurations (${config})." >&2
        echo "LLZK was built with headers at ${llzk_llvm_include}." >&2
        echo "use that LLVM/MLIR build, or rebuild LLZK against ${prefix}." >&2
        exit 1
      fi
    done
  done
fi
# Let the compiler select its matching SDK unless the caller overrides it.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-$(sw_vers -productVersion)}"

# Homebrew's LLVM uses system libraries such as zstd from its own prefix.
# Preserve a Nix/custom toolchain's flags without adding Homebrew libraries.
case "${LLVM_PREFIX}" in
  /opt/homebrew/*) brew_lib=/opt/homebrew/lib ;;
  /usr/local/*) brew_lib=/usr/local/lib ;;
  *) brew_lib= ;;
esac
if [[ -n "${brew_lib}" && "${RUSTFLAGS:-}" != *"-L ${brew_lib}"* && \
      "${RUSTFLAGS:-}" != *"-L${brew_lib}"* ]]; then
  export RUSTFLAGS="${RUSTFLAGS:-} -L ${brew_lib}"
fi

echo "Building with:"
echo "  MLIR_SYS_230_PREFIX=${MLIR_SYS_230_PREFIX}"
echo "  TABLEGEN_230_PREFIX=${TABLEGEN_230_PREFIX}"
echo "  LIBCLANG_PATH=${LIBCLANG_PATH}"
echo "  LLZK_SYS_30_PREFIX=${LLZK_SYS_30_PREFIX}"
echo "  SDKROOT=${SDKROOT:-<compiler default>}"
echo "  MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}"

cd "${repo_root}"
if [[ "$#" -eq 0 ]]; then
  cargo build
else
  cargo "$@"
fi
