#include "function.h"

#include "test.h"
#include "types/integers.h"

#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static firm_int_t *g_ints = NULL;
static uint32_t g_int_index = 0;
static uint32_t g_num_ints = 0;
static uint32_t g_block_num = 0;

void set_non_blocking_int_iterator(const uint32_t start, const uint32_t num,
                                   const uint32_t block_num) {
    g_int_index = 0;
    g_num_ints = num;
    g_block_num = block_num;
    g_ints = (firm_int_t *)malloc(sizeof(firm_int_t) * g_num_ints);

    for (uint32_t i = 0; i < g_num_ints; ++i) {
        g_ints[i] = start + i;
    }
}

void set_int_iterator(const firm_float_t start, const uint32_t num) {
    set_non_blocking_int_iterator(start, num, num);
}

#define MIN(a, b) ((a) < (b) ? (a) : (b))

const char *input_data_impl_int(const char *key, firm_size_t size,
                                ChannelData *result) {

    result->channel_type = ChannelDataType_Integer;
    uint32_t count = MIN((g_num_ints - g_int_index), size);

    // Simulate blocking call
    if (g_int_index + count > g_block_num) {
        result->count = 0;
        result->array = NULL;
        return NULL;
    }

    result->count = count;

    if (result->count != 0) {
        firm_int_t *array = malloc(sizeof(firm_int_t) * result->count);
        memcpy(array, &g_ints[g_int_index], sizeof(firm_int_t) * result->count);
        result->array = result->count == 0 ? NULL : array;
    }

    g_int_index += result->count;
    return NULL;
}

const char *input_data_impl_int_error(const char *key, firm_size_t size,
                                      ChannelData *result) {
    return create_string_ptr("Oh no!");
}

const char *input_available_impl_int(const char *key,
                                     firm_size_t *num_available_out,
                                     bool *closed_out) {
    *num_available_out = g_block_num - g_int_index;
    *closed_out = g_num_ints - g_int_index == 0;
    return NULL;
}

const char *input_type_impl_int(const char *key, firm_channel_type_t *result) {
    *result = ChannelDataType_Integer;
    return NULL;
}

void test_get_single_int() {
    set_int_iterator(42, 1);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    firm_int_t i;
    ApiResult res = ff_next_int("does-not-matter-here", true, &i);
    assert(ff_result_is_ok(&res));
    assert(i == 42);

    res = ff_next_int("does-not-matter-here", true, &i);
    assert(ff_result_is_end_of_input(&res));

    // Test errors
    set_input_data_impl(input_data_impl_int_error);
    res = ff_next_int("key", true, &i);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);

    free(g_ints);
}

void test_get_single_non_blocking_int() {
    set_non_blocking_int_iterator(1, 5, 1);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    firm_int_t i;
    ApiResult res = ff_next_int("does-not-matter-here", false, &i);
    assert(ff_result_is_ok(&res));

    res = ff_next_int("does-not-matter-here", false, &i);
    assert(ff_result_would_block(&res));
    free(g_ints);
}

void test_get_multiple_non_blocking_ints() {
    set_non_blocking_int_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    firm_int_t *int_array = NULL;
    firm_size_t num_ints = 0;
    ApiResult res = ff_ints("helbananer", false, 4, &int_array, &num_ints);
    assert(ff_result_is_ok(&res));
    free(int_array);

    res = ff_ints("helbananer", false, 4, &int_array, &num_ints);
    assert(ff_result_would_block(&res));

    free(g_ints);
}

void test_get_multiple_ints() {
    firm_int_t expected = 10;
    set_int_iterator(expected, 100);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    firm_int_t *int_array = NULL;
    firm_size_t num_ints = 0;
    ApiResult res = ff_ints("helatal", true, 50, &int_array, &num_ints);

    assert(ff_result_is_ok(&res));
    assert(num_ints == 50);

    for (firm_size_t i = 0; i < num_ints; ++i) {
        assert(int_array[i] == expected);
        ++expected;
    }

    free(int_array);

    // check that a closed input renders less than the requested amount
    res = ff_ints("helatal", true, 500, &int_array, &num_ints);

    assert(ff_result_is_ok(&res));
    assert(num_ints == 50);
    free(int_array);

    res = ff_ints("helatal", true, 500, &int_array, &num_ints);
    assert(ff_result_is_end_of_input(&res));

    free(g_ints);
}

void test_integer_iterator() {
    firm_int_t expected_int = 500;
    set_int_iterator(expected_int, 10);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    IntIterator *iterator = NULL;
    ApiResult res = ff_int_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_int_t i;
    uint32_t int_count = 0;
    ApiResult r;
    while ((r = ff_iterator_next_int(iterator, &i)).kind == ApiResult_Ok) {
        assert(expected_int == i);
        ++expected_int;
        ++int_count;
    }

    assert(int_count == 10);
    assert(ff_result_is_ok(&res));

    ff_close_int_iterator(iterator);
    free(g_ints);

    // Test error
    set_int_iterator(31415929, 11);
    set_input_data_impl(input_data_impl_int_error);

    iterator = NULL;
    res = ff_int_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    res = ff_iterator_next_int(iterator, &i);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);
    ff_close_int_iterator(iterator);

    free(g_ints);
}

void test_int_non_blocking_iterator() {
    set_non_blocking_int_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    IntIterator *iterator = NULL;
    ApiResult res =
        ff_int_iterator("does-not-matter-here", 4, false, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_int_t i;
    ApiResult r;
    assert(ff_iterator_next_int(iterator, &i).kind == ApiResult_Ok);
    assert(ff_iterator_next_int(iterator, &i).kind == ApiResult_Ok);
    assert(ff_iterator_next_int(iterator, &i).kind == ApiResult_Ok);
    assert(ff_iterator_next_int(iterator, &i).kind == ApiResult_Ok);
    assert(ff_iterator_next_int(iterator, &i).kind == ApiResult_Blocked);

    ff_close_int_iterator(iterator);
    free(g_ints);
}

void test_collect_integer_iterator() {
    firm_int_t expected_int = 500;
    set_int_iterator(expected_int, 10);
    set_input_data_impl(input_data_impl_int);
    set_channel_type_impl(input_type_impl_int);
    set_input_available_impl(input_available_impl_int);

    IntIterator *iterator = NULL;
    ApiResult res = ff_int_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_int_t *ints = NULL;
    firm_size_t num_ints = 0;
    res = ff_iterator_collect_ints(iterator, &ints, &num_ints);
    assert(ff_result_is_ok(&res));

    for (firm_size_t i = 0; i < num_ints; ++i) {
        assert(ints[i] == expected_int);
        ++expected_int;
    }

    ff_close_int_iterator(iterator);

    free(ints);
    free(g_ints);
}

firm_int_t *g_int_output_set = NULL;
bool g_int_output_closed = false;
const char *append_int_output_impl(const char *key, const ChannelData *values) {
    g_int_output_set = (firm_int_t *)values->array;

    return NULL;
}

const char *close_int_output_impl(const char *key) {
    g_int_output_closed = true;
    return NULL;
}

void test_int_output() {
    set_append_output_impl(append_int_output_impl);
    set_close_output_impl(close_int_output_impl);
    set_channel_type_impl(input_type_impl_int);

    firm_int_t ints[2] = {256, 7};

    ApiResult res = ff_append_int_output("siffror", ints, 2);
    assert(ff_result_is_ok(&res));
    assert(g_int_output_set[0] == 256);
    assert(g_int_output_set[1] == 7);

    res = ff_close_output("siffror");
    assert(ff_result_is_ok(&res));
    assert(g_int_output_closed);
}

void run_integer_tests() {
    run_test(test_get_single_int);
    /*run_test(test_get_single_non_blocking_int);
    run_test(test_get_multiple_non_blocking_ints); //
    run_test(test_get_multiple_ints);
    run_test(test_integer_iterator); //
    run_test(test_int_non_blocking_iterator);
    run_test(test_collect_integer_iterator);
    run_test(test_int_output);*/
}
