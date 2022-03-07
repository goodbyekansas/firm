#include "function.h"

#include "test.h"
#include "types/common.h"

#include <assert.h>
#include <math.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static firm_float_t *g_floats = NULL;
static uint32_t g_floats_index = 0;
static uint32_t g_num_floats = 0;
static uint32_t g_block_num = 0;

void set_non_blocking_float_iterator(const firm_float_t start,
                                     const uint32_t num,
                                     const uint32_t block_num) {
    g_floats_index = 0;
    g_num_floats = num;
    g_block_num = block_num;
    g_floats = (firm_float_t *)malloc(sizeof(firm_float_t) * g_num_floats);

    for (uint32_t i = 0; i < g_num_floats; ++i) {
        g_floats[i] = start + i;
    }
}

void set_float_iterator(const firm_float_t start, const uint32_t num) {
    set_non_blocking_float_iterator(start, num, num);
}

void reset_float_iterator() {
    free(g_floats);
    g_floats = NULL;
    g_floats_index = 0;
    g_num_floats = 0;
}

#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define ASSERT_EPSILON(a, b) assert(fabs(a - b) < 0.00001)

const char *input_data_impl_float(const char *key, firm_size_t size,
                                  ChannelData *result) {

    result->channel_type = ChannelDataType_Float;
    uint32_t count = MIN((g_num_floats - g_floats_index), size);

    // Simulate blocking call
    if (g_floats_index + count > g_block_num) {
        result->count = 0;
        result->array = NULL;
        return NULL;
    }

    result->count = count;
    if (result->count != 0) {
        firm_float_t *array = malloc(sizeof(firm_float_t) * result->count);
        memcpy(array, &g_floats[g_floats_index],
               sizeof(firm_float_t) * result->count);
        result->array = result->count == 0 ? NULL : array;
    }

    g_floats_index += result->count;
    return NULL;
}

const char *input_data_impl_float_error(const char *key, firm_size_t size,
                                        ChannelData *result) {
    return create_string_ptr("Oh no!");
}

const char *input_available_impl_float(const char *key,
                                       firm_size_t *num_available_out,
                                       bool *closed_out) {
    *num_available_out = g_block_num - g_floats_index;
    *closed_out = g_num_floats - g_floats_index == 0;
    return NULL;
}

const char *input_type_impl_float(const char *key,
                                  firm_channel_type_t *result) {
    *result = ChannelDataType_Float;
    return NULL;
}

void test_get_single_float() {
    set_float_iterator(42.5, 1);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    firm_float_t d = 0;
    ApiResult res = ff_next_float("does-not-matter-here", true, &d);
    assert(ff_result_is_ok(&res));
    assert(d == 42.5);

    res = ff_next_float("does-not-matter-here", true, &d);
    assert(ff_result_is_end_of_input(&res));

    // Test errors
    set_input_data_impl(input_data_impl_float_error);
    res = ff_next_float("key", true, &d);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);

    reset_float_iterator();
}

void test_get_single_non_blocking_float() {
    set_non_blocking_float_iterator(1, 5, 1);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    firm_float_t f;
    ApiResult res = ff_next_float("does-not-matter-here", false, &f);
    assert(ff_result_is_ok(&res));

    res = ff_next_float("does-not-matter-here", false, &f);
    assert(ff_result_would_block(&res));
    free(g_floats);
}

void test_get_multiple_non_blocking_floats() {
    set_non_blocking_float_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    firm_float_t *float_array = NULL;
    firm_size_t num_floats = 0;
    ApiResult res =
        ff_floats("flytbananer", false, 4, &float_array, &num_floats);
    assert(ff_result_is_ok(&res));
    free(float_array);

    res = ff_floats("flytbananer", false, 4, &float_array, &num_floats);
    assert(ff_result_would_block(&res));

    free(g_floats);
}

void test_get_multiple_floats() {
    firm_float_t expected = 10.11;
    set_float_iterator(expected, 10);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    firm_float_t *float_array = NULL;
    firm_size_t num_floats = 0;
    ApiResult res =
        ff_floats("flytandetal", true, 4, &float_array, &num_floats);

    assert(ff_result_is_ok(&res));
    assert(num_floats == 4);

    for (firm_size_t i = 0; i < num_floats; ++i) {
        ASSERT_EPSILON(float_array[i], expected);
        expected += 1.0;
    }

    free(float_array);

    // check that a closed input renders less than the requested amount
    res = ff_floats("flytandetal", true, 500, &float_array, &num_floats);

    assert(ff_result_is_ok(&res));
    assert(num_floats == 6);
    free(float_array);

    res = ff_floats("flytandetal", true, 500, &float_array, &num_floats);
    assert(ff_result_is_end_of_input(&res));

    free(g_floats);
}

void test_float_iterator() {
    set_float_iterator(500.5, 10);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    FloatIterator *iterator = NULL;
    ApiResult res =
        ff_float_iterator("does-not-matter-here", 4, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_float_t d;
    firm_float_t expected_float = 500.5;
    ApiResult r;
    while ((r = ff_iterator_next_float(iterator, &d)).kind == ApiResult_Ok) {
        ASSERT_EPSILON(expected_float, d);
        ++expected_float;
    }

    assert(ff_result_is_end_of_input(&r));

    ff_close_float_iterator(iterator);
    free(g_floats);

    // Test error
    set_float_iterator(3.1415929, 11);
    set_input_data_impl(input_data_impl_float_error);
    iterator = NULL;
    res = ff_float_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    res = ff_iterator_next_float(iterator, &d);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);
    ff_close_float_iterator(iterator);

    free(g_floats);
}

void test_float_non_blocking_iterator() {
    set_non_blocking_float_iterator(1, 7, 6);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    FloatIterator *iterator = NULL;
    ApiResult res =
        ff_float_iterator("does-not-matter-here", 4, false, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_float_t f;
    ApiResult r;
    assert(ff_iterator_next_float(iterator, &f).kind == ApiResult_Ok);
    assert(ff_iterator_next_float(iterator, &f).kind == ApiResult_Ok);
    assert(ff_iterator_next_float(iterator, &f).kind == ApiResult_Ok);
    assert(ff_iterator_next_float(iterator, &f).kind == ApiResult_Ok);
    assert(ff_iterator_next_float(iterator, &f).kind == ApiResult_Blocked);

    ff_close_float_iterator(iterator);
    free(g_floats);
}

void test_collect_float_iterator() {
    firm_float_t expected_float = 500.444;
    set_float_iterator(expected_float, 10);
    set_input_data_impl(input_data_impl_float);
    set_channel_type_impl(input_type_impl_float);
    set_input_available_impl(input_available_impl_float);

    FloatIterator *iterator = NULL;
    ApiResult res =
        ff_float_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_float_t *floats = NULL;
    firm_size_t num_floats = 0;
    res = ff_iterator_collect_floats(iterator, &floats, &num_floats);
    assert(ff_result_is_ok(&res));

    for (firm_size_t i = 0; i < num_floats; ++i) {
        ASSERT_EPSILON(floats[i], expected_float);
        ++expected_float;
    }

    ff_close_float_iterator(iterator);

    free(floats);
    free(g_floats);
}

firm_float_t *g_float_output_set = NULL;
bool g_float_output_closed = false;
const char *append_float_output_impl(const char *key,
                                     const ChannelData *values) {
    g_float_output_set = (firm_float_t *)values->array;

    return NULL;
}

const char *close_float_output_impl(const char *key) {
    g_float_output_closed = true;
    return NULL;
}

void test_float_output() {
    set_append_output_impl(append_float_output_impl);
    set_close_output_impl(close_float_output_impl);
    set_channel_type_impl(input_type_impl_float);

    double floats[2] = {256.5, 7.5};

    ApiResult res = ff_append_float_output("flytande", floats, 2);
    assert(ff_result_is_ok(&res));
    ASSERT_EPSILON(g_float_output_set[0], floats[0]);
    ASSERT_EPSILON(g_float_output_set[1], floats[1]);

    res = ff_close_output("flytande");
    assert(ff_result_is_ok(&res));
    assert(g_float_output_closed);
}

void run_float_tests() {
    run_test(test_get_single_float);
    run_test(test_get_single_non_blocking_float);
    run_test(test_get_multiple_floats);
    run_test(test_get_multiple_non_blocking_floats);
    run_test(test_float_iterator);
    run_test(test_float_non_blocking_iterator);
    run_test(test_collect_float_iterator);
    run_test(test_float_output);
}
