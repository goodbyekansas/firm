#include <assert.h>
#include <stdbool.h>
#include <stdio.h>

#include "test.h"

#define HOST_FN_IMPL(name, passargs, ...)                                      \
    static name##_fn g_##name##_impl = NULL;                                   \
    void set_##name##_impl(name##_fn impl) { g_##name##_impl = impl; }         \
    const char *__##name(__VA_ARGS__) {                                        \
        if (g_##name##_impl == NULL) {                                         \
            fprintf(stderr, #name " implementation not set\n");                \
            assert(false);                                                     \
        }                                                                      \
        return g_##name##_impl passargs;                                       \
    }

HOST_FN_IMPL(input_data, (key, count, result), const char *key,
             firm_size_t count, ChannelData *result);
HOST_FN_IMPL(channel_type, (key, type_out), const char *key,
             firm_channel_type_t *type_out);
HOST_FN_IMPL(channel_closed, (key, closed_out), const char *key,
             bool *closed_out);
HOST_FN_IMPL(input_available, (key, num_out, closed_out), const char *key,
             firm_size_t *num_out, bool *closed_out);
HOST_FN_IMPL(append_output, (key, data), const char *key,
             const ChannelData *data);
HOST_FN_IMPL(close_output, (key), const char *key);
HOST_FN_IMPL(map_attachment, (name, unpack, path_out), const char *name,
             bool unpack, const char **path_out);
HOST_FN_IMPL(host_path_exists, (path, exists_out), const char *path,
             bool *exists_out);
HOST_FN_IMPL(host_os, (os_out), const char **os_out);
HOST_FN_IMPL(start_host_process, (request, pid_out, exit_code_out),
             const StartProcessRequest *request, uint64_t *pid_out,
             int64_t *exit_code_out);
HOST_FN_IMPL(set_error, (error), const char *error);
HOST_FN_IMPL(connect, (addr, file_descriptor_out), const char *addr,
             int32_t *file_descriptor_out);
