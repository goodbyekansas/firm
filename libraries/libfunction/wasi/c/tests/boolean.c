#include "function.h"

#include "test.h"

#include <assert.h>
#include <math.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static firm_bool_t *g_booleans = NULL;
static uint32_t g_booleans_index = 0;
static uint32_t g_num_booleans = 0;
static uint32_t g_block_num = 0;

void set_non_blocking_bool_iterator(const uint32_t num,
                                    const uint32_t block_num) {
    g_booleans_index = 0;
    g_num_booleans = num;
    g_block_num = block_num;
    g_booleans = (bool *)malloc(sizeof(bool) * g_num_booleans);

    for (uint32_t i = 0; i < g_num_booleans; ++i) {
        g_booleans[i] = i % 2 == 0;
    }
}

void set_bool_iterator(const uint32_t num) {
    set_non_blocking_bool_iterator(num, num);
}

#define MIN(a, b) ((a) < (b) ? (a) : (b))

const char *input_data_impl_bool(const char *key, firm_size_t size,
                                 ChannelData *result) {
    uint32_t count = MIN((g_num_booleans - g_booleans_index), size);
    result->channel_type = ChannelDataType_Boolean;

    // Simulate blocking call
    if (g_booleans_index + count > g_block_num) {
        result->count = 0;
        result->array = NULL;
        return NULL;
    }

    result->count = count;
    if (result->count != 0) {
        firm_bool_t *array = malloc(sizeof(firm_bool_t) * result->count);
        memcpy(array, &g_booleans[g_booleans_index],
               sizeof(firm_bool_t) * result->count);
        result->array = result->count == 0 ? NULL : array;
    }

    g_booleans_index += result->count;
    return NULL;
}

const char *input_data_impl_bool_error(const char *key, firm_size_t size,
                                       ChannelData *result) {
    return create_string_ptr("Oh no!");
}

const char *input_available_impl_bool(const char *key,
                                      firm_size_t *num_available_out,
                                      bool *closed_out) {
    *num_available_out = g_block_num - g_booleans_index;
    *closed_out = g_num_booleans - g_booleans_index == 0;
    return NULL;
}

const char *input_type_impl_bool(const char *key, firm_channel_type_t *result) {
    *result = ChannelDataType_Boolean;
    return NULL;
}

void test_get_single_bool() {
    set_bool_iterator(1);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    firm_bool_t b;
    ApiResult res = ff_next_bool("does-not-matter-here", true, &b);
    assert(ff_result_is_ok(&res));
    assert(b == true);

    res = ff_next_bool("does-not-matter-here", true, &b);
    assert(ff_result_is_end_of_input(&res));

    // Test errors
    set_input_data_impl(input_data_impl_bool_error);
    res = ff_next_bool("key", true, &b);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);

    free(g_booleans);
}

void test_get_single_non_blocking_bool() {
    set_non_blocking_bool_iterator(5, 1);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    firm_bool_t b;
    ApiResult res = ff_next_bool("does-not-matter-here", false, &b);
    assert(ff_result_is_ok(&res));

    res = ff_next_bool("does-not-matter-here", false, &b);
    assert(ff_result_would_block(&res));
    free(g_booleans);
}

void test_get_multiple_non_blocking_bools() {
    set_non_blocking_bool_iterator(7, 6);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    firm_bool_t *bool_array = NULL;
    firm_size_t num_bools = 0;
    ApiResult res = ff_bools("boolebananer", false, 4, &bool_array, &num_bools);
    assert(ff_result_is_ok(&res));
    free(bool_array);

    res = ff_bools("boolebananer", false, 4, &bool_array, &num_bools);
    assert(ff_result_would_block(&res));

    free(g_booleans);
}

void test_get_multiple_bools() {
    set_bool_iterator(0xf + 1);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    firm_bool_t *bool_array = NULL;
    firm_size_t num_bools = 0;
    ApiResult res =
        ff_bools("boolebananer", true, 014, &bool_array, &num_bools);

    assert(ff_result_is_ok(&res));
    assert(num_bools == 0xc);

    firm_size_t count = 0;
    for (firm_size_t i = 0; i < num_bools; ++i) {
        assert(bool_array[i] == ((count % 2) == 0));
        ++count;
    }

    free(bool_array);

    // check that a closed input renders less than the requested amount
    res = ff_bools("bool-banan", true, 500, &bool_array, &num_bools);

    assert(ff_result_is_ok(&res));
    assert(num_bools == 04);
    free(bool_array);

    res = ff_bools("bool-klot", true, 500, &bool_array, &num_bools);
    assert(ff_result_is_end_of_input(&res));

    free(g_booleans);
}

void test_bool_iterator() {
    set_bool_iterator(10);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    BoolIterator *iterator = NULL;
    ApiResult res =
        ff_bool_iterator("does-not-matter-here", 4, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_bool_t b;
    firm_size_t count = 0;
    ApiResult r;
    while ((r = ff_iterator_next_bool(iterator, &b)).kind == ApiResult_Ok) {
        assert(b == ((count % 2) == 0));
        ++count;
    }

    assert(ff_result_is_end_of_input(&r));

    ff_close_bool_iterator(iterator);
    free(g_booleans);

    // Test error
    set_bool_iterator(11);
    set_input_data_impl(input_data_impl_bool_error);
    iterator = NULL;
    res = ff_bool_iterator("does-not-matter-here", 2, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    res = ff_iterator_next_bool(iterator, &b);
    assert(ff_result_is_err(&res));
    assert(strstr(res.error_msg, "Oh no!") != NULL);
    ff_close_bool_iterator(iterator);

    free(g_booleans);
}

void test_bool_non_blocking_iterator() {
    set_non_blocking_bool_iterator(7, 6);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    BoolIterator *iterator = NULL;
    ApiResult res =
        ff_bool_iterator("does-not-matter-here", 4, false, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_bool_t b;
    ApiResult r;
    assert(ff_iterator_next_bool(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_bool(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_bool(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_bool(iterator, &b).kind == ApiResult_Ok);
    assert(ff_iterator_next_bool(iterator, &b).kind == ApiResult_Blocked);

    ff_close_bool_iterator(iterator);
    free(g_booleans);
}

void test_collect_bool_iterator() {
    // land of a thousand bools
    set_bool_iterator(1000);
    set_input_data_impl(input_data_impl_bool);
    set_channel_type_impl(input_type_impl_bool);
    set_input_available_impl(input_available_impl_bool);

    BoolIterator *iterator = NULL;
    ApiResult res =
        ff_bool_iterator("does-not-matter-here", 100, true, &iterator);

    assert(ff_result_is_ok(&res));
    assert(iterator != NULL);

    firm_bool_t *bools = NULL;
    firm_size_t num_bools = 0;
    res = ff_iterator_collect_bools(iterator, &bools, &num_bools);
    assert(ff_result_is_ok(&res));

    firm_size_t count = 0;
    for (firm_size_t i = 0; i < num_bools; ++i) {
        assert(bools[i] == ((count % 2) == 0));
        ++count;
    }

    ff_close_bool_iterator(iterator);

    free(bools);
    free(g_booleans);
}

firm_bool_t *g_bool_output_set = NULL;
bool g_bool_output_closed = false;
const char *append_bool_output_impl(const char *key,
                                    const ChannelData *values) {
    g_bool_output_set = (bool *)values->array;

    return NULL;
}

const char *close_bool_output_impl(const char *key) {
    g_bool_output_closed = true;
    return NULL;
}

void test_bool_output() {
    set_append_output_impl(append_bool_output_impl);
    set_close_output_impl(close_bool_output_impl);
    set_channel_type_impl(input_type_impl_bool);

    firm_bool_t bools[] = {true, false};

    ApiResult res = ff_append_bool_output("booleaner", bools, 2);
    assert(ff_result_is_ok(&res));
    assert(g_bool_output_set[0] == true);
    assert(g_bool_output_set[1] == false);

    res = ff_close_output("booleananer 🍌");
    assert(ff_result_is_ok(&res));
    assert(g_bool_output_closed);
}

void run_bool_tests() {
    run_test(test_get_single_bool);
    run_test(test_get_single_non_blocking_bool);
    run_test(test_get_multiple_non_blocking_bools);
    run_test(test_get_multiple_bools);
    run_test(test_bool_iterator);
    run_test(test_bool_non_blocking_iterator);
    run_test(test_collect_bool_iterator);
    run_test(test_bool_output);
}
