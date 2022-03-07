#include "function.h"

#include "test.h"

#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static firm_string_t *g_strings = NULL;
static uint32_t g_string_index = 0;
static uint32_t g_num_strings = 0;
static uint32_t g_block_num = 0;

void set_non_blocking_string_iterator(firm_string_t template,
                                      const uint32_t num_strings,
                                      const uint32_t block_num) {
    g_string_index = 0;
    g_num_strings = num_strings;
    g_block_num = block_num;
    g_strings = (firm_string_t *)malloc(sizeof(firm_string_t) * g_num_strings);

    for (uint32_t i = 0; i < num_strings; ++i) {
        const uint32_t n = (strlen(template) + 32);
        firm_string_t s = malloc(sizeof(char) * n);
        snprintf((char *)s, n, "%s-%d", template, i);
        g_strings[i] = s;
    }
}

void free_g_strings() { //🩲
    for (uint32_t i = 0; i < g_num_strings; ++i) {
        free(g_strings[i]);
    }

    free(g_strings);
}

void set_string_iterator(firm_string_t template, const uint32_t num_strings) {
    set_non_blocking_string_iterator(template, num_strings, num_strings);
}

#define MIN(a, b) ((a) < (b) ? (a) : (b))

const char *input_data_impl_string(const char *key, firm_size_t size,
                                   ChannelData *result) {

    result->channel_type = ChannelDataType_String;
    uint32_t count = MIN((g_num_strings - g_string_index), size);

    // Simulate blocking call
    if (g_string_index + count > g_block_num) {
        result->count = 0;
        result->array = NULL;
        return NULL;
    }

    result->count = count;
    if (result->count != 0) {
        firm_string_t *array = malloc(sizeof(firm_string_t) * result->count);

        for (uint32_t i = 0; i < result->count; ++i) {
            uint32_t len = strlen(g_strings[i + g_string_index]) + 1;
            firm_string_t str = malloc(sizeof(char) * len);
            strncpy(str, g_strings[i + g_string_index], len);
            array[i] = str;
        }

        result->array = result->count == 0 ? NULL : array;
    }

    g_string_index += result->count;
    return NULL;
}

const char *input_data_impl_string_error(const char *key, firm_size_t size,
                                         ChannelData *result) {
    return create_string_ptr("Things went poorly 🤡");
}

const char *input_available_impl_string(const char *key,
                                        firm_size_t *num_available_out,
                                        bool *closed_out) {
    *num_available_out = g_block_num - g_string_index;
    *closed_out = g_num_strings - g_string_index == 0;
    return NULL;
}

const char *input_type_impl_string(const char *key, uint8_t *result) {
    *result = ChannelDataType_String;
    return NULL;
}

void free_string_array(firm_string_t *strings, firm_size_t num) {
    for (firm_size_t i = 0; i < num; ++i) {
        free(strings[i]);
    }
    free(strings);
}

void test_get_single_string() {
    set_string_iterator("I am string", 1);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    firm_string_t s = NULL;
    ApiResult res = ff_next_string("does-not-matter-here", true, &s);
    assert(ff_result_is_ok(&res));
    assert(strncmp(s, "I am string-0", 13) == 0);
    free(s);

    res = ff_next_string("does-not-matter-here", true, &s);
    assert(ff_result_is_end_of_input(&res));

    // Test errors
    set_input_data_impl(input_data_impl_string_error);
    res = ff_next_string("key", true, &s);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Things went poorly 🤡") != NULL);

    free_g_strings();
}

void test_get_single_non_blocking_string() {
    set_non_blocking_string_iterator("blecking", 5, 1);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    firm_string_t s;
    ApiResult res = ff_next_string("does-not-matter-here", false, &s);
    assert(ff_result_is_ok(&res));
    free(s);

    res = ff_next_string("does-not-matter-here", false, &s);
    assert(ff_result_would_block(&res));
    free_g_strings();
}

void test_get_multiple_non_blocking_string() {
    set_non_blocking_string_iterator("blecking", 7, 6);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    firm_string_t *string_array = NULL;
    firm_size_t num_strings = 0;
    ApiResult res =
        ff_strings("strängbananer", false, 4, &string_array, &num_strings);
    assert(ff_result_is_ok(&res));
    free_string_array(string_array, num_strings);

    res = ff_strings("strängbananer", false, 4, &string_array, &num_strings);
    assert(ff_result_would_block(&res));

    free_g_strings();
}

void test_get_multiple_strings() {
    set_string_iterator("I am string", 10);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    firm_string_t *string_array = NULL;
    firm_size_t num_strings = 0;
    ApiResult res =
        ff_strings("strängar", true, 5, &string_array, &num_strings);

    assert(ff_result_is_ok(&res));
    assert(num_strings == 5);

    firm_size_t index = 0;
    const uint32_t check_len = 32 + 12;
    char check[check_len];
    for (firm_size_t i = 0; i < num_strings; ++i) {
        snprintf(check, check_len, "%s-%d", "I am string", index);
        assert(strncmp(string_array[i], check, strlen(check)) == 0);
        ++index;
    }
    free_string_array(string_array, num_strings);

    // check that a closed input renders less than the requested amount
    res = ff_strings("strängar", true, 50, &string_array, &num_strings);

    assert(ff_result_is_ok(&res));
    assert(num_strings == 5);
    free_string_array(string_array, num_strings);

    res = ff_strings("strängar", true, 50, &string_array, &num_strings);
    assert(ff_result_is_end_of_input(&res));

    free_g_strings();
}

void test_get_string_iterator() {
    set_string_iterator("I am string", 10);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    StringIterator *iterator = NULL;
    ApiResult res =
        ff_string_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_string_t s = NULL;
    uint32_t index = 0;
    const uint32_t check_len = 32 + 12;
    char check[check_len];
    ApiResult r;
    while ((r = ff_iterator_next_string(iterator, &s)).kind == ApiResult_Ok) {
        snprintf(check, check_len, "%s-%d", "I am string", index);
        assert(strncmp(s, check, strlen(check)) == 0);
        ++index;
        free(s);
    }

    assert(index == 10);
    assert(ff_result_is_end_of_input(&r));

    ff_close_string_iterator(iterator);
    free_g_strings();

    // Test error
    set_string_iterator("String am I", 11);
    set_input_data_impl(input_data_impl_string_error);
    iterator = NULL;
    res = ff_string_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    res = ff_iterator_next_string(iterator, &s);
    assert(ff_result_is_err(&res));
    ff_close_string_iterator(iterator);

    assert(strstr(res.error_msg, "Things went poorly 🤡") != NULL);

    free_g_strings();
}

void test_string_non_blocking_iterator() {
    set_non_blocking_string_iterator("blecking", 7, 6);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    StringIterator *iterator = NULL;
    ApiResult res =
        ff_string_iterator("does-not-matter-here", 4, false, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_string_t s;
    ApiResult r;
    assert(ff_iterator_next_string(iterator, &s).kind == ApiResult_Ok);
    free(s);
    assert(ff_iterator_next_string(iterator, &s).kind == ApiResult_Ok);
    free(s);
    assert(ff_iterator_next_string(iterator, &s).kind == ApiResult_Ok);
    free(s);
    assert(ff_iterator_next_string(iterator, &s).kind == ApiResult_Ok);
    free(s);
    assert(ff_iterator_next_string(iterator, &s).kind == ApiResult_Blocked);

    ff_close_string_iterator(iterator);
    free_g_strings();
}

void test_collect_string_iterator() {
    char *expected = "I am strängmannen";
    set_string_iterator(expected, 10);
    set_input_data_impl(input_data_impl_string);
    set_channel_type_impl(input_type_impl_string);
    set_input_available_impl(input_available_impl_string);

    StringIterator *iterator = NULL;
    ApiResult res =
        ff_string_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_string_t *string_array = NULL;
    firm_size_t num_strings = 0;
    res = ff_iterator_collect_strings(iterator, &string_array, &num_strings);
    assert(ff_result_is_ok(&res));

    uint32_t index = 0;
    const uint32_t check_len = 32 + 12;
    char check[check_len];
    for (firm_size_t i = 0; i < num_strings; ++i) {
        snprintf(check, check_len, "%s-%d", expected, index);
        assert(strncmp(string_array[i], check, strlen(check)) == 0);
        ++index;
    }

    free_string_array(string_array, num_strings);
    ff_close_string_iterator(iterator);
    free_g_strings();
}

firm_string_t *g_string_output_set = NULL;
bool g_string_output_closed = false;
const char *append_string_output_impl(const char *key,
                                      const ChannelData *values) {
    g_string_output_set = (firm_string_t *)values->array;
    return NULL;
}

const char *close_string_output_impl(const char *key) {
    g_string_output_closed = true;
    return NULL;
}

void test_string_output() {
    set_append_output_impl(append_string_output_impl);
    set_close_output_impl(close_string_output_impl);
    set_channel_type_impl(input_type_impl_string);

    firm_string_t strings[] = {"0-string", "1-string"};

    ApiResult res = ff_append_string_output("kej", strings, 2);
    assert(ff_result_is_ok(&res));
    assert(strncmp(g_string_output_set[0], strings[0], strlen(strings[0])) ==
           0);
    assert(strncmp(g_string_output_set[1], strings[1], strlen(strings[1])) ==
           0);

    res = ff_close_output("kej");
    assert(ff_result_is_ok(&res));
    assert(g_string_output_closed);
}

void run_string_tests() {
    run_test(test_get_single_string);
    run_test(test_get_single_non_blocking_string);
    run_test(test_get_multiple_non_blocking_string);
    run_test(test_get_multiple_strings);
    run_test(test_get_string_iterator);
    run_test(test_string_non_blocking_iterator);
    run_test(test_collect_string_iterator);
    run_test(test_string_output);
}
