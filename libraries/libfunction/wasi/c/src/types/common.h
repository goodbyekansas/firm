/*! \defgroup Common types for interacting with the functions API.
 * @{
 */

#ifndef COMMON_H
#define COMMON_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FIRM_SIZE_MAX UINT32_MAX;

typedef uint32_t firm_size_t;

typedef char *firm_string_t;
typedef int64_t firm_int_t;
typedef double firm_float_t;
typedef bool firm_bool_t;
typedef uint8_t firm_byte_t;
typedef uint8_t firm_channel_type_t;

enum ApiResultKind {
    ApiResult_Ok = 0,
    ApiResult_Blocked,
    ApiResult_EndOfInput,
    ApiResult_Error,
};

typedef struct ApiResult {
    uint8_t kind;
    const char *error_msg;
} ApiResult;

bool ff_result_is_ok(const ApiResult *result);
bool ff_result_is_err(const ApiResult *result);
bool ff_result_would_block(const ApiResult *result);
bool ff_result_is_end_of_input(const ApiResult *result);

#ifdef __cplusplus
}
#endif
#endif

/*! @} */
