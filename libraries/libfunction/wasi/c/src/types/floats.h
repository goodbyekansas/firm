/*! \defgroup Doubles Functions for accessing double inputs and outputs
 * @{
 */
#ifndef FLOATS_H
#define FLOATS_H

#include <stdbool.h>
#include <stdint.h>

#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/*! \brief An iterator over data of a float input */
typedef struct FloatIterator FloatIterator;

/*! \brief Get a single input as a float.
 *
 * \param[in] key Name of the input to fetch.
 * \param[in] blocking True if this function is allowed to block waiting for
 * data. \param[out] result On a successful fetch, *result will be set to the
 * first float value in the channel.
 *
 * Note that if the input contains more than a single value, this will only
 * return the first one.
 */
ApiResult ff_next_float(const char *key, bool blocking, firm_float_t *result);

/*! \brief Get many inputs as an array
 *
 * \param[in] key Name of the input to fetch.
 * \param[in] blocking True if this function is allowed to block waiting for
 * data.
 * \param[in] size The number of elements you wish to grab.
 * \param[out] result On a successful fetch, *result will be set to the
 * array.
 * \param[out] size_out Number of items in the output array. Can be smaller than
 * size.
 *
 */
ApiResult ff_floats(const char *key, bool blocking, firm_size_t size,
                    firm_float_t **result, firm_size_t *size_out);

/*! \brief Get an iterator over a float input.
 *
 * \param[in] key Name of the input to fetch.
 * \param[in] fetch_size If new data needs to be fetched, fetch this amount.
 * \param[in] blocking True if this iterator is allowed to block waiting for
 * data.
 * \param[out] result When successful, *result will be set to a
 * FloatIterator.
 *
 */
ApiResult ff_float_iterator(const char *key, firm_size_t fetch_size,
                            bool blocking, FloatIterator **result);

/*! \brief Get the next float in the iterator.
 *
 * \param[in] iter The FloatInputIterator to iterate over.
 * \param[out] result If successfull, *result will be set to the next float.
 *
 * Note that there may be more inputs that aren't readily available.
 * Depending on if the iterator is set to blocking or not you will get
 * different behaviour. If the iterator is blocking it may take some time
 * before it yields the next value. If it's not blocking the function will
 * return false without setting any result. You can check if the last call
 * was blocking by calling the `last_call_blocked()` function.
 */
ApiResult ff_iterator_next_float(const FloatIterator *iter,
                                 firm_float_t *result);

/*! \brief Collect all floats in this input iterator
 *
 * \param[in] iter The FloatIterator to collect
 * \param[out] result A pointer that for a succesful collect will be set to
 * point to a contigous array.
 * \param[out] num_out Size of the resulting array
 */
ApiResult ff_iterator_collect_floats(FloatIterator *iter, firm_float_t **result,
                                     firm_size_t *num_out);

/*! \brief Appends floats to an output.
 *
 * \param[in] key Name of the output to append.
 * \param[in] floats The floats you want to append to the output.
 * \param[in] num_floats The number of floats you want to add to the output.
 */
ApiResult ff_append_float_output(const char *key, const firm_float_t *floats,
                                 firm_size_t num_floats);

/*! \brief Closes the iterator
 *
 * \param[in] iter The FloatInputIterator to close.
 */
void ff_close_float_iterator(FloatIterator *iter);

#ifdef __cplusplus
}
#endif
#endif

/*! @} */
