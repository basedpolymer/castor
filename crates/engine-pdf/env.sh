# Source before building engine-pdf by hand: `. crates/engine-pdf/env.sh` (Git Bash / macOS).
_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd -W 2>/dev/null || pwd)"
export BINDGEN_EXTRA_CLANG_ARGS="-D__CLANG_MAX_ALIGN_T_DEFINED -include ${_here}/bindgen/max_align_t.h"
if [ -d "/c/Program Files/LLVM/bin" ]; then export LIBCLANG_PATH="C:/Program Files/LLVM/bin"; fi
# MuPDF's Visual Studio projects compile with /GL (whole-program optimisation): every link of a
# binary that contains MuPDF then re-generates code for the whole 600 MB library (2–3 min per
# test binary or CLI link). -GL- keeps normal object files; links take seconds. Takes effect the
# next time mupdf-sys is (re)built.
export _CL_="-GL-"  # dash form: Git Bash would rewrite "/GL-" into a Windows path
