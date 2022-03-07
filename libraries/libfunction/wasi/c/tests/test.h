#ifndef TEST_H
#define TEST_H

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "function.h"

typedef struct ChannelData {
    uint8_t channel_type;
    uint32_t count;
    const void *array;
} ChannelData;

enum ChannelDataType {
    ChannelDataType_Null = 0,
    ChannelDataType_String = 1,
    ChannelDataType_Integer = 2,
    ChannelDataType_Float = 3,
    ChannelDataType_Boolean = 4,
    ChannelDataType_Byte = 5
};

#define HOST_FN(fn_name, ...)                                                  \
    typedef const char *(*fn_name##_fn)(__VA_ARGS__);                          \
    void set_##fn_name##_impl(fn_name##_fn impl);

HOST_FN(input_data, const char *key, firm_size_t count, ChannelData *result);
HOST_FN(channel_type, const char *key, firm_channel_type_t *type_out);
HOST_FN(channel_closed, const char *key, bool *closed_out);
HOST_FN(input_available, const char *key, firm_size_t *num_out,
        bool *closed_out);
HOST_FN(append_output, const char *key, const ChannelData *data);
HOST_FN(close_output, const char *key);
HOST_FN(map_attachment, const char *name, bool unpack, const char **path_out);
HOST_FN(host_path_exists, const char *path, bool *exists_out);
HOST_FN(host_os, const char **os_out);
HOST_FN(start_host_process, const StartProcessRequest *request,
        uint64_t *pid_out, int64_t *exit_code_out);
HOST_FN(set_error, const char *error);
HOST_FN(connect, const char *addr, int32_t *file_descriptor_out);

#define run_test(fn)                                                           \
    printf("    🧜 running \033[1;36m" #fn "\033[0m... ");                   \
    fflush(stdout);                                                            \
    fn();                                                                      \
    printf("\033[32mok!\033[0m\n");

void run_common_tests();
void run_string_tests();
void run_integer_tests();
void run_float_tests();
void run_bool_tests();
void run_byte_tests();

static const char *create_string_ptr(const char *s) {
    char *mem = malloc(sizeof(char) * (strlen(s) + 1));
    strncpy(mem, s, strlen(s) + 1);
    return mem;
}

#endif
