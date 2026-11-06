# Source before building engine-pdf by hand: `. crates/engine-pdf/env.sh` (Git Bash / macOS).
_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd -W 2>/dev/null || pwd)"
export BINDGEN_EXTRA_CLANG_ARGS="-D__CLANG_MAX_ALIGN_T_DEFINED -include ${_here}/bindgen/max_align_t.h"
if [ -d "/c/Program Files/LLVM/bin" ]; then export LIBCLANG_PATH="C:/Program Files/LLVM/bin"; fi
