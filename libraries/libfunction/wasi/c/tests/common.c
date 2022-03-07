#include "function.h"

#include "test.h"

#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

const char *map_attachment_impl(const char *name, bool unpack,
                                const char **path_out) {
    *path_out = create_string_ptr("/data/pata");
    return NULL;
}

const char *map_attachment_error_impl(const char *name, bool unpack,
                                      const char **path_out) {
    *path_out = NULL;
    return create_string_ptr("Sad times");
}

void test_map_attachment() {
    set_map_attachment_impl(map_attachment_impl);
    const char *path = NULL;
    ApiResult res = ff_map_attachment("attackment", true, &path);
    assert(ff_result_is_ok(&res));
    assert(strncmp(path, "/data/pata", 10) == 0);
    free((void *)path);

    // Test error behaviour
    set_map_attachment_impl(map_attachment_error_impl);
    res = ff_map_attachment("attackment", true, &path);
    assert(ff_result_is_err(&res));
    assert(strncmp(res.error_msg, "Sad times", 9));
}

const char *host_path_exists_impl(const char *path, bool *exists_out) {
    *exists_out = strncmp(path, "exists", 6) == 0;
    return NULL;
}

void test_host_path_exists() {
    set_host_path_exists_impl(host_path_exists_impl);
    bool yes = false;
    ApiResult res = ff_host_path_exists("exists", &yes);
    assert(ff_result_is_ok(&res));
    assert(yes);
    res = ff_host_path_exists("ExIsTs", &yes);
    assert(ff_result_is_ok(&res));
    assert(!yes);
}

const char *get_host_os_impl(const char **os_out) {
    *os_out = create_string_ptr("solaris");
    return NULL;
}

void test_get_host_os() {
    set_host_os_impl(get_host_os_impl);
    char *os = NULL;
    ApiResult result = ff_get_host_os(&os);
    assert(ff_result_is_ok(&result));
    assert(strncmp(os, "solaris", 7) == 0);
    free(os);
}

const char *start_host_process_impl(const StartProcessRequest *request,
                                    uint64_t *pid_out, int64_t *exit_code_out) {
    if (request->wait) {
        *exit_code_out = 0;
    }

    *pid_out = 1337;
    return NULL;
}

void test_start_host_process() {
    set_start_host_process_impl(start_host_process_impl);

    // Test normal call.
    uint64_t pid;
    int64_t exit_code = 5;
    StartProcessRequest request = {
        .command = "Sune tog all sås.",
        .env_vars = NULL,
        .num_env_vars = 0,
        .wait = true,
    };

    ApiResult res = ff_start_host_process(&request, &pid, &exit_code);
    assert(ff_result_is_ok(&res));
    assert(exit_code == 0);
    assert(pid == 1337);

    // Test that exit code is untouched if not waiting.
    exit_code = 17;
    request.wait = false;
    res = ff_start_host_process(&request, &pid, &exit_code);
    assert(ff_result_is_ok(&res));
    assert(exit_code == 17);
}

static const char *g_error_message = NULL;
const char *set_error_impl(const char *message) {
    g_error_message = message;
    return NULL;
}

void test_set_function_error() {
    set_set_error_impl(set_error_impl);
    const char *message = "Jag gillar att mosa potatisen med såsen.";
    ApiResult res = ff_set_function_error(message);
    assert(ff_result_is_ok(&res));
    assert(strncmp(g_error_message, message, strlen(message)) == 0);
}

const char *connect_impl(const char *address, int32_t *file_descriptor) {
    *file_descriptor = 5;
    return NULL;
}

const char *connect_error_impl(const char *address, int32_t *file_descriptor) {
    return create_string_ptr("no space of jam");
}

void test_connect() {
    set_connect_impl(connect_impl);
    int32_t file_descriptor;
    ApiResult res = ff_connect("jam-of-space.com", &file_descriptor);
    assert(ff_result_is_ok(&res));
    assert(file_descriptor == 5);

    // Test error
    set_connect_impl(connect_error_impl);
    res = ff_connect("jam-of-space.com", &file_descriptor);
    assert(ff_result_is_err(&res));
    assert(strncmp(res.error_msg, "no space of jam", 0xf));
}

void run_common_tests() {
    run_test(test_map_attachment);
    run_test(test_host_path_exists);
    run_test(test_get_host_os);
    run_test(test_start_host_process);
    run_test(test_set_function_error);
    run_test(test_connect);
}
