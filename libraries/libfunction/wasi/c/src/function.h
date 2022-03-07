/*! \defgroup Main Functions for accessing type-invariant WASI host
 * functionality
 * @{
 */
#ifndef FUNCTION_H
#define FUNCTION_H

#include <stdbool.h>
#include <stdint.h>

#include "types/bools.h"
#include "types/bytes.h"
#include "types/common.h"
#include "types/floats.h"
#include "types/integers.h"
#include "types/strings.h"

#ifdef __cplusplus
extern "C" {
#endif

/*! \brief Close an output for writing.
 *
 * After this, no more data can be appended to the output.
 */
ApiResult ff_close_output(const char *key);

/*! \brief Maps an attachmet to the specified path. Unpacks archive if
 * requested.
 *
 * \param[in] attachment_name The name of the attachment to map.
 * \param[in] unpack Bool telling if it should try to unpack (only works if file
 * is an archive).
 * \param[out] path_out Path to the location where you can
 * access the mapped attachment.
 */
ApiResult ff_map_attachment(const char *attachment_name, bool unpack,
                            char **path_out);

/*! \brief Checks whether a path on the host platform exists.
 *
 * \param[in] path The path to a file or folder.
 * \param[out] exists_out True if the path exists. False if the path does not
 * exist.
 */
ApiResult ff_host_path_exists(const char *path, bool *exists_out);

/*! \brief Gets a string with the name of the host OS.
 *
 * \param[out] os_out A string with the name of the host os.
 *
 * The host os string can be one of the following.
 * - "linux"
 * - "macos"
 * - "ios"
 * - "freebsd"
 * - "dragonfly"
 * - "netbsd"
 * - "openbsd"
 * - "solaris"
 * - "android"
 * - "windows"
 */
ApiResult ff_get_host_os(char **os_out);

typedef struct EnvironmentVariable {
    const char *key;
    const char *value;
} EnvironmentVariable;

typedef struct StartProcessRequest {
    const char *command;
    const EnvironmentVariable *env_vars;
    uint32_t num_env_vars;
    bool wait;
} StartProcessRequest;

/*! \brief Starts a host process with the supplied command and gives you the
 * pid.
 *
 * \param[in] command A string with the command for starting a process.
 * \param[in] env_vars An array of EnvironmentVariables for the process.
 * \param[in] wait Wait for the process to exit. If this is true, both the pid
 * and the exit code will be set. If it is false, only the pid will be set.
 * \param[out] pid_out The pid the started process got.
 */
ApiResult ff_start_host_process(const StartProcessRequest *request,
                                uint64_t *pid_out, int64_t *exit_code_out);

/*! \brief Sets an error message for the function. Replaces existing message if
 * any.
 *
 * \param[in] message A string containing the error message.
 */
ApiResult ff_set_function_error(const char *message);

/*! \brief Connects to a socket through a file descriptor.
 *
 * \param[in] address The address to connect to.
 * \param[in] file_descriptor The resulting socket file descriptor.
 */
ApiResult ff_connect(const char *address, int32_t *file_descriptor);

#ifdef __cplusplus
}
#endif
#endif

/*! @} */
