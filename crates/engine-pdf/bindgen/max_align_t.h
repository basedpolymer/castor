/* Work-around for mupdf-sys 0.8.0 on MSVC (and Apple): clang declares max_align_t there as a
 * typedef of a primitive type, which bindgen drops when it is marked opaque, and the `mupdf`
 * crate then fails to compile. Pre-declaring a struct with the same size and alignment makes
 * bindgen emit it. Used only while generating bindings:
 *   BINDGEN_EXTRA_CLANG_ARGS="-D__CLANG_MAX_ALIGN_T_DEFINED -include <this file>"
 */
#if defined(_MSC_VER)
typedef struct { double castor_d; } max_align_t;
#else
typedef struct { long long castor_ll; long double castor_ld; } max_align_t;
#endif
