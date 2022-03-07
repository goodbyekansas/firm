#include "function.h"

#include "test.h"
#include "types/common.h"

#include <assert.h>
#include <math.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static firm_byte_t *g_bytes = NULL;
static uint32_t g_bytes_index = 0;
static uint32_t g_num_bytes = 0;
static uint32_t g_block_num = 0;

void set_non_blocking_byte_iterator(const firm_byte_t start, const uint32_t num,
                                    const uint32_t block_num) {
    g_bytes_index = 0;
    g_num_bytes = num;
    g_block_num = block_num;
    g_bytes = (firm_byte_t *)malloc(sizeof(firm_byte_t) * g_num_bytes);

    for (uint32_t i = 0; i < g_num_bytes; ++i) {
        g_bytes[i] = (start + i) & 0xff;
    }
}

void set_byte_iterator(const firm_byte_t start, const uint32_t num) {
    set_non_blocking_byte_iterator(start, num, num);
}

#define MIN(a, b) ((a) < (b) ? (a) : (b))

const char *input_data_impl_byte(const char *key, firm_size_t size,
                                 ChannelData *result) {

    uint32_t count = MIN((g_num_bytes - g_bytes_index), size);
    result->channel_type = ChannelDataType_Boolean;

    // Simulate blocking call
    if (g_bytes_index + count > g_block_num) {
        result->count = 0;
        result->array = NULL;
        return NULL;
    }

    result->count = count;
    if (result->count != 0) {
        firm_byte_t *array = malloc(sizeof(firm_byte_t) * result->count);
        memcpy(array, &g_bytes[g_bytes_index],
               sizeof(firm_byte_t) * result->count);
        result->array = result->count == 0 ? NULL : array;
    }

    g_bytes_index += result->count;
    return NULL;
}

const char *input_data_impl_byte_error(const char *key, firm_size_t size,
                                       ChannelData *result) {
    return create_string_ptr("Oh no!");
}

const char *input_available_impl_byte(const char *key,
                                      firm_size_t *num_available_out,
                                      bool *closed_out) {
    *num_available_out = g_block_num - g_bytes_index;
    *closed_out = g_num_bytes - g_bytes_index == 0;
    return NULL;
}

const char *input_type_impl_byte(const char *key, firm_channel_type_t *result) {
    *result = ChannelDataType_Byte;
    return NULL;
}

void test_get_single_byte() {
    set_byte_iterator(42, 1);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    firm_byte_t b = 0;
    ApiResult res = ff_next_byte("does-not-matter-here", true, &b);
    assert(ff_result_is_ok(&res));
    assert(b == 42);

    res = ff_next_byte("does-not-matter-here", true, &b);
    assert(ff_result_is_end_of_input(&res));

    // Test errors
    set_input_data_impl(input_data_impl_byte_error);
    res = ff_next_byte("key", true, &b);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);

    free(g_bytes);
}

void test_get_single_non_blocking_byte() {
    set_non_blocking_byte_iterator(1, 5, 1);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    firm_byte_t b;
    ApiResult res = ff_next_byte("does-not-matter-here", false, &b);
    assert(ff_result_is_ok(&res));

    res = ff_next_byte("does-not-matter-here", false, &b);
    assert(ff_result_would_block(&res));
    free(g_bytes);
}

void test_get_multiple_non_blocking_bytes() {
    set_non_blocking_byte_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    firm_byte_t *byte_array = NULL;
    firm_size_t num_bytes = 0;
    ApiResult res = ff_bytes("bajtbananer", false, 4, &byte_array, &num_bytes);
    assert(ff_result_is_ok(&res));
    free(byte_array);

    res = ff_bytes("bajtbananer", false, 4, &byte_array, &num_bytes);
    assert(ff_result_would_block(&res));

    free(g_bytes);
}

void test_get_multiple_bytes() {
    firm_byte_t expected = 10;
    set_byte_iterator(expected, 100);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    firm_byte_t *byte_array = NULL;
    firm_size_t num_bytes = 0;
    ApiResult res = ff_bytes("bett", true, 50, &byte_array, &num_bytes);

    assert(ff_result_is_ok(&res));
    assert(num_bytes == 50);

    for (firm_size_t i = 0; i < num_bytes; ++i) {
        assert(byte_array[i] == expected);
        ++expected;
    }

    free(byte_array);

    // check that a closed input renders less than the requested amount
    res = ff_bytes("bett", true, 500, &byte_array, &num_bytes);

    assert(ff_result_is_ok(&res));
    assert(num_bytes == 50);
    free(byte_array);

    res = ff_bytes("bett", true, 500, &byte_array, &num_bytes);
    assert(ff_result_is_end_of_input(&res));

    free(g_bytes);
}

void test_byte_iterator() {
    set_byte_iterator(128, 10);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    ByteIterator *iterator = NULL;
    ApiResult res =
        ff_byte_iterator("does-not-matter-here", 4, true, &iterator);

    assert(ff_result_is_ok(&res));

    firm_byte_t b;
    firm_byte_t expected_byte = 128;
    while ((res = ff_iterator_next_byte(iterator, &b)).kind == ApiResult_Ok) {
        assert(b == expected_byte);
        ++expected_byte;
    }

    assert(ff_result_is_end_of_input(&res));

    ff_close_byte_iterator(iterator);
    free(g_bytes);

    // Test error
    set_byte_iterator(3, 11);
    set_input_data_impl(input_data_impl_byte_error);
    iterator = NULL;
    res = ff_byte_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    res = ff_iterator_next_byte(iterator, &b);
    assert(ff_result_is_err(&res));
    ff_close_byte_iterator(iterator);

    assert(strstr(res.error_msg, "Oh no!") != NULL);
    free(g_bytes);
}

void test_byte_non_blocking_iterator() {
    set_non_blocking_byte_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    ByteIterator *iterator = NULL;
    ApiResult res =
        ff_byte_iterator("does-not-matter-here", 4, false, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_byte_t b;
    ApiResult r;
    assert(ff_iterator_next_byte(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_byte(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_byte(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_byte(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_byte(iterator, &b).kind == ApiResult_Blocked);

    ff_close_byte_iterator(iterator);
    free(g_bytes);
}

void test_collect_byte_iterator() {
    firm_byte_t expected_byte = 128;
    set_byte_iterator(expected_byte, 10);
    set_input_data_impl(input_data_impl_byte);
    set_channel_type_impl(input_type_impl_byte);
    set_input_available_impl(input_available_impl_byte);

    ByteIterator *iterator = NULL;
    ApiResult res =
        ff_byte_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_byte_t *bytes = NULL;
    firm_size_t num_bytes = 0;
    res = ff_iterator_collect_bytes(iterator, &bytes, &num_bytes);
    assert(ff_result_is_ok(&res));

    for (firm_size_t i = 0; i < num_bytes; ++i) {
        assert(bytes[i] == expected_byte);
        ++expected_byte;
    }

    ff_close_byte_iterator(iterator);

    free(bytes);
    free(g_bytes);
}

firm_byte_t *g_byte_output_set = NULL;
bool g_byte_output_closed = false;
const char *append_byte_output_impl(const char *key,
                                    const ChannelData *values) {
    g_byte_output_set = (firm_byte_t *)values->array;

    return NULL;
}

const char *close_byte_output_impl(const char *key) {
    g_byte_output_closed = true;
    return NULL;
}

void test_byte_output() {
    set_append_output_impl(append_byte_output_impl);
    set_close_output_impl(close_byte_output_impl);
    set_channel_type_impl(input_type_impl_byte);

    firm_byte_t bytes[] = {64, 128};

    ApiResult res = ff_append_byte_output("underbett", bytes, 2);
    assert(ff_result_is_ok(&res));
    assert(g_byte_output_set[0] == 64);
    assert(g_byte_output_set[1] == 128);

    res = ff_close_output("överbett");
    assert(ff_result_is_ok(&res));
    assert(g_byte_output_closed);
}

void run_byte_tests() {
    run_test(test_get_single_byte);
    run_test(test_get_single_non_blocking_byte);
    run_test(test_get_multiple_non_blocking_bytes);
    run_test(test_get_multiple_bytes);
    run_test(test_byte_iterator);
    run_test(test_byte_non_blocking_iterator);
    run_test(test_collect_byte_iterator);
    run_test(test_byte_output);
}
