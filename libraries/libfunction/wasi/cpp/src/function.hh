#ifndef FUNCTION_HH
#define FUNCTION_HH

#include <algorithm>
#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <memory>
#include <ostream>
#include <stdexcept>
#include <string>
#include <unistd.h>
#include <utility>
#include <vector>

namespace host {
#include <firm/function.h>
}

namespace firm {

/*! \brief Class representing a result from the function API. */
template <class ReturnType> class ApiResult {

public:
  /*! \brief Create an ApiResult representing a successful operation.
   *
   * \param [in] result the result of the operation.
   */
  static ApiResult<ReturnType> ok(ReturnType result) {
    return ApiResult<ReturnType>(std::move(result));
  }

  /*! \brief Create an ApiResult representing a unsuccessful operation.
   *
   * \param [in] error_message the error message for the failed operation.
   */
  static ApiResult<ReturnType> err(const char *error_message) {
    return ApiResult<ReturnType>(error_message);
  }

  /*! \brief Create an ApiResult representing a unsuccessful operation.
   *
   * \param [in] error_message the error message for the failed operation.
   */
  static ApiResult<ReturnType> err(std::string &error_message) {
    return ApiResult<ReturnType>(error_message);
  }

  ApiResult(ApiResult<ReturnType> &&other) noexcept {
    *this = std::move(other);
  }

  ~ApiResult<ReturnType>() {
    if (_ok) {
      _result.~ReturnType();
    } else {
      _error_message.std::string::~string();
    }
  }

  ApiResult<ReturnType> &operator=(ApiResult<ReturnType> &&rhs) noexcept {
    _ok = rhs._ok;
    if (_ok) {
      _result = std::move(rhs._result);
    } else {
      _error_message = rhs._error_message;
    }

    return *this;
  }

  friend std::ostream &operator<<(std::ostream &os,
                                  const ApiResult<ReturnType> &res) {
    if (res._ok) {
      os << "Ok";
    } else {
      os << "Err(\"" << res._error_message << "\")";
    }

    return os;
  }

  /*! \brief True if this ApiResult represents an unsuccessful API operation
   */
  bool is_error() { return !_ok; }

  /*! \brief True if this ApiResult represents a successful API operation
   */
  bool is_ok() { return _ok; }

  /*! \brief Obtain a reference to the contained result of a successful API
   * operation
   */
  ReturnType &value() { return _result; }

  /*! \brief Obtain a reference to the error message associated with an
   *  unsuccessful API operation
   */
  std::string &error_message() { return _error_message; }

private:
  ApiResult(ReturnType result) : _ok(true), _result(std::move(result)) {}

  ApiResult(const char *error_message)
      : _ok(false), _error_message(error_message) {}

  bool _ok;
  union {
    ReturnType _result;
    std::string _error_message;
  };
};

template <> class ApiResult<void> {
public:
  static ApiResult<void> ok() { return ApiResult<void>(); }
  static ApiResult<void> err(const char *error_message) {
    return ApiResult<void>(error_message);
  }

  bool is_error() { return !_ok; }

  bool is_ok() { return _ok; }

  std::string &error_message() { return _error_message; }

private:
  ApiResult() : _ok(true) {}
  ApiResult(const char *error_message)
      : _ok(false), _error_message(error_message) {}

  bool _ok;
  std::string _error_message;
};

/*! \brief Interface for wrapping the C-style iterator with type information */
template <typename DataType> class IInputIterator {
public:
  virtual ~IInputIterator() {}
  virtual std::pair<const DataType, bool> next_value() = 0;
};

/*! \brief Class representing a stream of input values for a single input.*/
template <typename DataType> class InputValues {
public:
  InputValues(std::shared_ptr<IInputIterator<DataType>> &iter) : _iter(iter) {}

  class iterator {
  public:
    friend class InputValues;
    iterator &operator++() {
      auto next = _inner->next_value();
      if (next.second) {
        _curr = next.first;
      } else {
        _end = true;
      }

      return *this;
    }

    bool operator==(const iterator &other) const {
      return this->_end == other._end;
    }

    bool operator!=(const iterator &other) const { return !(*this == other); }

    const DataType &operator*() const { return _curr; }

  private:
    iterator(std::shared_ptr<IInputIterator<DataType>> inner)
        : _inner(inner), _end(false) {
      // fetch potential first value, c++ iterators (sensibly) expect
      // begin() to give an iterator that yields the first value in the
      // collection. However, we need to call next_* on our C iterator
      // before that happens, i.e. our begin() points _before_ the first
      // value
      ++(*this);
    }

    iterator() : _end(true), _inner(nullptr) {}
    bool _end;
    std::shared_ptr<IInputIterator<DataType>> _inner;
    DataType _curr;
  };

  iterator begin() { return iterator(_iter); }
  iterator end() { return iterator(); }

private:
  std::shared_ptr<IInputIterator<DataType>> _iter;
};

/*! \brief Interface for the C host API.
 *
 * Main purpose of this is to make it possible to mock calls to the C API for
 * testing purposes.
 */
class IHostApi {
public:
  virtual ~IHostApi(){};

  /* Common */
  virtual host::ApiResult close_output(const char *key) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult map_attachment(const char *attachment_name,
                                         bool unpack, char **path_out) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult host_path_exists(const char *path, bool *exists_out) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult get_host_os(char **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult
  start_host_process(const host::StartProcessRequest *request,
                     uint64_t *pid_out, int64_t *exit_code_out) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult connect(const char *address,
                                  int32_t *file_descriptor) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult set_function_error(const char *message) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  /* Strings */
  virtual host::ApiResult get_string_input(const char *key, bool blocking,
                                           host::firm_string_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult get_string_iterator(const char *key,
                                              host::firm_size_t fetch_size,
                                              bool blocking,
                                              host::StringIterator **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult next_string(host::StringIterator *iter,
                                      char **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual void close_string_iterator(host::StringIterator *iter) {}

  virtual host::ApiResult get_int_input(const char *key, bool blocking,
                                        host::firm_int_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  /* Ints */
  virtual host::ApiResult get_int_iterator(const char *key,
                                           host::firm_size_t fetch_size,
                                           bool blocking,
                                           host::IntIterator **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult next_int(host::IntIterator *iter,
                                   host::firm_int_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual void close_int_iterator(host::IntIterator *iter) {}

  /* Floats */
  virtual host::ApiResult get_float_input(const char *key, bool blocking,
                                          host::firm_float_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult get_float_iterator(const char *key,
                                             host::firm_size_t fetch_size,
                                             bool blocking,
                                             host::FloatIterator **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult next_float(host::FloatIterator *iter,
                                     host::firm_float_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual void close_float_iterator(host::FloatIterator *iter) {}

  /* Bools */
  virtual host::ApiResult get_bool_input(const char *key, bool blocking,
                                         host::firm_bool_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult get_bool_iterator(const char *key,
                                            host::firm_size_t fetch_size,
                                            bool blocking,
                                            host::BoolIterator **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult next_bool(host::BoolIterator *iter,
                                    host::firm_bool_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual void close_bool_iterator(host::BoolIterator *iter) {}

  /* Bytes */
  virtual host::ApiResult get_byte_input(const char *key, bool blocking,
                                         host::firm_byte_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult get_byte_iterator(const char *key,
                                            host::firm_size_t fetch_size,
                                            bool blocking,
                                            host::ByteIterator **result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual host::ApiResult next_byte(host::ByteIterator *iter,
                                    host::firm_byte_t *result) {
    return host::ApiResult{host::ApiResult_Error, "Not Implemented!"};
  }

  virtual void close_byte_iterator(host::ByteIterator *iter) {}
};

/*! \brief Implementation of the "hot" path, that does real api calls */
class RealApiImpl : public IHostApi {
#ifndef TESTING

  /* Common */
  virtual host::ApiResult close_output(const char *key) override {
    return host::ff_close_output(key);
  }

  virtual host::ApiResult map_attachment(const char *attachment_name,
                                         bool unpack,
                                         char **path_out) override {
    return host::ff_map_attachment(attachment_name, unpack, path_out);
  }

  virtual host::ApiResult host_path_exists(const char *path,
                                           bool *exists_out) override {
    return host::ff_host_path_exists(path, exists_out);
  }

  virtual host::ApiResult get_host_os(char **result) override {
    return host::ff_get_host_os(result);
  }

  virtual host::ApiResult
  start_host_process(const host::StartProcessRequest *request,
                     uint64_t *pid_out, int64_t *exit_code_out) override {
    return host::ff_start_host_process(request, pid_out, exit_code_out);
  }

  virtual host::ApiResult set_function_error(const char *message) override {
    return host::ff_set_function_error(message);
  }

  virtual host::ApiResult connect(const char *address,
                                  int32_t *file_descriptor) override {
    return host::ff_connect(address, file_descriptor);
  }

  /* Strings */
  virtual host::ApiResult get_string_input(const char *key, bool blocking,
                                           char **result) override {
    return host::ff_next_string(key, blocking, (host::firm_string_t *)result);
  }

  virtual host::ApiResult
  get_string_iterator(const char *key, host::firm_size_t fetch_size,
                      bool blocking, host::StringIterator **result) override {
    return host::ff_string_iterator(key, fetch_size, blocking, result);
  }

  virtual host::ApiResult next_string(host::StringIterator *iter,
                                      char **result) override {
    return host::ff_iterator_next_string(iter, result);
  }

  virtual void close_string_iterator(host::StringIterator *iter) override {
    return host::ff_close_string_iterator(iter);
  }

  /* Ints */
  virtual host::ApiResult get_int_input(const char *key, bool blocking,
                                        host::firm_int_t *result) override {
    return host::ff_next_int(key, blocking, result);
  }

  virtual host::ApiResult
  get_int_iterator(const char *key, host::firm_size_t fetch_size, bool blocking,
                   host::IntIterator **result) override {
    return host::ff_int_iterator(key, fetch_size, blocking, result);
  }

  virtual host::ApiResult next_int(host::IntIterator *iter,
                                   host::firm_int_t *result) override {
    return host::ff_iterator_next_int(iter, result);
  }

  virtual void close_int_iterator(host::IntIterator *iter) override {
    host::ff_close_int_iterator(iter);
  }

  /* Floats */
  virtual host::ApiResult get_float_input(const char *key, bool blocking,
                                          host::firm_float_t *result) override {
    return host::ff_next_float(key, blocking, result);
  }

  virtual host::ApiResult
  get_float_iterator(const char *key, host::firm_size_t fetch_size,
                     bool blocking, host::FloatIterator **result) override {
    return host::ff_float_iterator(key, fetch_size, blocking, result);
  }

  virtual host::ApiResult next_float(host::FloatIterator *iter,
                                     host::firm_float_t *result) override {
    return host::ff_iterator_next_float(iter, result);
  }

  virtual void close_float_iterator(host::FloatIterator *iter) override {
    host::ff_close_float_iterator(iter);
  }

  /* Bools */
  virtual host::ApiResult get_bool_input(const char *key, bool blocking,
                                         host::firm_bool_t *result) override {
    return host::ff_next_bool(key, blocking, result);
  }

  virtual host::ApiResult
  get_bool_iterator(const char *key, host::firm_size_t fetch_size,
                    bool blocking, host::BoolIterator **result) override {
    return host::ff_bool_iterator(key, fetch_size, blocking, result);
  }

  virtual host::ApiResult next_bool(host::BoolIterator *iter,
                                    host::firm_bool_t *result) override {
    return host::ff_iterator_next_bool(iter, result);
  }

  virtual void close_bool_iterator(host::BoolIterator *iter) override {
    host::ff_close_bool_iterator(iter);
  }

  /* Bytes */
  virtual host::ApiResult get_byte_input(const char *key, bool blocking,
                                         host::firm_byte_t *result) override {
    return host::ff_next_byte(key, blocking, result);
  }

  virtual host::ApiResult
  get_byte_iterator(const char *key, host::firm_size_t fetch_size,
                    bool blocking, host::ByteIterator **result) override {
    return host::ff_byte_iterator(key, fetch_size, blocking, result);
  }

  virtual host::ApiResult next_byte(host::ByteIterator *iter,
                                    host::firm_byte_t *result) override {
    return host::ff_iterator_next_byte(iter, result);
  }

  virtual void close_byte_iterator(host::ByteIterator *iter) override {
    host::ff_close_byte_iterator(iter);
  }
#endif
};

static std::unique_ptr<IHostApi> g_impl =
    std::make_unique<IHostApi>(RealApiImpl());

/*! \brief Set the backend API implementation.
 *
 * This is supposed to be used for testing purposes, primarily.
 * \param [in] impl An implementation of the IHostApi implementation.
 */
inline void set_api_impl(IHostApi *impl) { g_impl.reset(impl); }

/* Common */
/*! \brief Close an output, making it impossible to write more data.
 *
 * A closed output will not produce more data, functions using this output as
 * input will then know that no more input is coming.
 *
 * \param [in] key The name of the output to close.
 * \return An empty ApiResult indicating the outcome of the operation.
 */
inline ApiResult<void> close_output(const std::string &key) {
  auto api_result = g_impl->close_output(key.c_str());
  if (host::ff_result_is_ok(&api_result)) {
    return ApiResult<void>::ok();
  } else {
    return ApiResult<void>::err(api_result.error_msg);
  }
}

/*! \brief Map a function attachment to a path in the function runtime
 * environment.
 *
 * \param [in] key The name of the attachment to map.
 * \param [in] unpack If the attachment is an archive, unpack if this is true.
 * \return An ApiResult indicating the outcome of the operation. On success,
 * contains a string representing the path the attachment was mapped to in the
 * function runtime environment.
 */
inline ApiResult<std::string> map_attachment(const std::string &key,
                                             bool unpack) {
  char *path_out = nullptr;
  auto api_result = g_impl->map_attachment(key.c_str(), unpack, &path_out);

  if (host::ff_result_is_ok(&api_result)) {
    auto str_path_out = std::string(path_out);
    std::free(path_out);
    return ApiResult<std::string>::ok(str_path_out);
  } else {
    return ApiResult<std::string>::err(api_result.error_msg);
  }
}

/*! \brief Check if a path exists on the host executing the function.
 *
 * \param [in] path The path on the host to check.
 * \return An ApiResult indicating the outcome of the operation. On success,
 * contains a bool indicating whether the path exists or not.
 */
inline ApiResult<bool> host_path_exists(const std::string &path) {
  bool exists = false;
  auto api_result = g_impl->host_path_exists(path.c_str(), &exists);

  if (host::ff_result_is_ok(&api_result)) {
    return ApiResult<bool>::ok(exists);
  } else {
    return ApiResult<bool>::err(api_result.error_msg);
  }
}

/*! \brief Get the operating system name of the host executing the function.
 *
 * \return An ApiResult indicating the outcome of the operation. On success,
 * containing string with the name of the operating system.
 */
inline ApiResult<std::string> get_host_os() {
  char *os_out = nullptr;
  auto result = g_impl->get_host_os(&os_out);

  if (host::ff_result_is_ok(&result)) {
    auto os_str = std::string(os_out);
    std::free(os_out);
    return ApiResult<std::string>::ok(os_str);
  } else {
    return ApiResult<std::string>::err(result.error_msg);
  }
}

/*! \brief Class representing a process run on the host executing the function.
 *
 * The class represent the state of the process when the host function was
 * called and is not updated after that. Any later change in the process state
 * will not be reflected here.
 */
class HostProcess {
public:
  /*! \brief The pid of the process when it is/was running.*/
  uint64_t pid() { return _pid; }

  /*! \brief The exit code of the process when it when it exited, note that
   * calling this function on a running process is undefined behavior.
   */
  int64_t exit_code() { return _exit_code; }

  /*! \brief If the process exited before control was handed back to the
   * function, indicating if exit code or pid are useful.
   */
  bool exited() { return _exited; }

private:
  HostProcess(uint64_t pid, int64_t exit_code, bool exited)
      : _pid(pid), _exit_code(exit_code), _exited(exited) {}
  uint64_t _pid;
  int64_t _exit_code;
  bool _exited;

  friend class HostProcessBuilder;
};

/*! \brief Class for setting up and starting a process on the host executing the
 * function.*/
class HostProcessBuilder {
public:
  /*! \brief Construct a HostProcessBuilder with the command to run.
   *
   * \param [in] command The command to run, including arguments.
   */
  HostProcessBuilder(const std::string &command) : _command(command) {}

  /*! \brief Set environment variables for the process.
   *
   * This function can be called multiple times to extend the existing
   * environment.
   *
   * \param [in] env_vars The map of environment variables to add.
   */
  HostProcessBuilder &
  environment_variables(const std::map<std::string, std::string> &env_vars) {
    _environment_variables.insert(env_vars.begin(), env_vars.end());
    return *this;
  }

  /*! \brief Set whether to wait for the process to exit or not.
   * \param [in] wait True if it should wait.
   */
  HostProcessBuilder &wait(bool wait) {
    _wait = wait;
    return *this;
  }

  /*! \brief Start the process.
   * \return An ApiResult indicating the outcome of the operation. On success,
   * contains a HostProcess with status of the process.
   */
  ApiResult<HostProcess> start() {
    uint64_t pid_out = 0;
    int64_t exit_code_out = 0;
    host::EnvironmentVariable *envs =
        static_cast<host::EnvironmentVariable *>(std::malloc(
            sizeof(host::EnvironmentVariable) * _environment_variables.size()));

    std::size_t index = 0;
    for (const auto &kv : _environment_variables) {
      envs[index].key = kv.first.c_str();
      envs[index].value = kv.second.c_str();
      ++index;
    }

    host::StartProcessRequest request = {
        .command = _command.c_str(),
        .env_vars = envs,
        .num_env_vars = (uint32_t)index,
        .wait = _wait,
    };

    auto api_result =
        g_impl->start_host_process(&request, &pid_out, &exit_code_out);

    std::free(envs);

    if (host::ff_result_is_ok(&api_result)) {
      return ApiResult<HostProcess>::ok(
          HostProcess(pid_out, exit_code_out, _wait));
    } else {
      return ApiResult<HostProcess>::err(api_result.error_msg);
    }
  }

private:
  std::string _command;
  std::map<std::string, std::string> _environment_variables;
  bool _wait;
};

/*! \brief Set the function to error state with a message.
 *
 * This will not abort the function but after the function exists it will be
 * considered failed.
 *
 * \param [in] error_message The error message.
 * \return An empty ApiResult indicating the outcome of the operation, on
 * failure to set the error containing that error message.
 */
inline ApiResult<void> set_function_error(const std::string &error_message) {
  auto api_result = g_impl->set_function_error(error_message.c_str());

  if (host::ff_result_is_ok(&api_result)) {
    return ApiResult<void>::ok();
  } else {
    return ApiResult<void>::err(api_result.error_msg);
  }
}

/*! \brief Class representing a socket address, a combination of hostname and
 * port.*/
class SocketAddress {
public:
  /*! \brief Parse a SocketAddress from a string.
   *
   * The parsing does validity checking, that the host name is not an ip address
   * and it has a port.
   *
   * \param [in] address The string representation of the address.
   * \return An ApiResult indicating the outcome of the operation. On success,
   * contains SocketAddress.
   */
  static ApiResult<SocketAddress> parse(const std::string &address) {
    if (!has_port(address)) {
      return ApiResult<SocketAddress>::err("Address must contain a port.");
    }

    if (!is_hostname(address)) {
      return ApiResult<SocketAddress>::err(
          "Address must be a hostname (Ipv4 or Ipv6 addresses are not "
          "allowed).");
    }

    return ApiResult<SocketAddress>::ok(SocketAddress(address));
  }

  /*! \brief A string representation of the SocketAdress*/
  const std::string &address() { return _address; }

private:
  SocketAddress(const std::string &address) : _address(address) {}

  static bool is_hostname(const std::string &address) {
    return !is_ipv4(address) && !is_ipv6(address);
  }

  static bool has_port(const std::string &address) {

    // Ipv6 case
    if (address.find("]:") != std::string::npos) {
      return true;
    }

    // Ipv4 case
    if (address.find(":", address.find(".")) != std::string::npos) {
      return true;
    }

    // hostname case
    if (is_hostname(address) && address.find(":") != std::string::npos) {
      return true;
    }

    return false;
  }

  static bool is_ipv4(const std::string &address) {
    auto without_port = address.substr(0, address.find(":"));
    auto dot_count = std::count(without_port.begin(), without_port.end(), '.');

    if (dot_count != 3) {
      return false;
    }

    auto token = without_port;
    for (size_t i = 0; i < 4; i++) {

      auto index = token.find(".");

      auto value = std::stoi(token.substr(0, index));
      if (value < 0 || value > 255) {
        return false;
      }

      token = token.substr(index + 1, token.length());
    }

    return true;
  }

  static bool is_ipv6(const std::string &address) {
    // We always expect to have a port in the address. Ipv6 addresses
    // always need to have the following format with a port
    // [Ipv6]:port which is why we can make this assumption.
    return address[0] == '[' && address.find("]") != std::string::npos;
  }

  std::string _address;
};

/*! \brief Class representing a TCP or UDP connection to a socket.*/
class SocketConnection {
public:
  enum Protocol {
    TCP,
    UDP,
  };

  SocketConnection(SocketConnection &&rhs) noexcept
      : _address(std::move(rhs._address)),
        _file_descriptor(std::move(rhs._file_descriptor)) {
    rhs._file_descriptor = -1;
  }

  SocketConnection &operator=(SocketConnection &&rhs) noexcept {
    _address = std::move(rhs._address);
    _file_descriptor = std::move(rhs._file_descriptor);

    rhs._file_descriptor = -1;

    return *this;
  }

  ~SocketConnection() {
    if (_file_descriptor >= 0) {
      close(_file_descriptor);
    }
  }

  /*! \brief Create a new connection.
   * \param [in] protocol The Protocol to use.
   * \param [in] address A pair of address and port.
   * \return An ApiResult indicating the outcome of the operation, on success
   * contains a SocketConnection that can be used to sending an receiving data
   * to/from the socket.
   */
  static ApiResult<SocketConnection>
  connect(Protocol protocol,
          const std::pair<const std::string, const std::uint16_t> &address) {
    std::string address_port = address.first;
    if (address_port.find(":") != std::string::npos) {
      address_port = "[" + address_port + "]";
    }

    address_port = address_port + ":" + std::to_string(address.second);
    return SocketConnection::connect(protocol, address_port);
  }

  /*! \brief Create a new connection.
   * \param [in] protocol The Protocol to use.
   * \param [in] address A string representation of address and port.
   * \return An ApiResult indicating the outcome of the operation, on success
   * contains a SocketConnection that can be used to sending an receiving data
   * to/from the socket.
   */
  static ApiResult<SocketConnection> connect(Protocol protocol,
                                             const std::string &address) {
    auto address_result = SocketAddress::parse(address);
    if (address_result.is_error()) {
      return ApiResult<SocketConnection>::err(
          address_result.error_message().c_str());
    }

    return SocketConnection::connect(protocol, address_result.value());
  }

  /*! \brief Create a new connection.
   * \param [in] protocol The Protocol to use.
   * \param [in] address The SocketAddress to connect to.
   * \return An ApiResult indicating the outcome of the operation, on success
   * contains a SocketConnection that can be used to sending an receiving data
   * to/from the socket.
   */
  static ApiResult<SocketConnection> connect(Protocol protocol,
                                             SocketAddress &address) {
    auto protocol_str = protocol == Protocol::TCP ? "tcp://" : "udp://";

    int32_t file_descriptor;
    auto api_result = g_impl->connect(
        (protocol_str + address.address()).c_str(), &file_descriptor);

    if (host::ff_result_is_ok(&api_result)) {
      return ApiResult<SocketConnection>::ok(
          SocketConnection(address, file_descriptor));
    } else {
      return ApiResult<SocketConnection>::err(api_result.error_msg);
    }
  }

  /*! \brief Send data to the socket.
   * \param [in] data The data to send.
   * \return An ApiResult indicating the outcome of the operation, on success
   * contains amount of bytes written to the socket.
   */
  ApiResult<size_t> send(const std::vector<uint8_t> &data) const {
    auto written_bytes = write(_file_descriptor, data.data(), data.size());
    if (written_bytes > 0) {
      return ApiResult<size_t>::ok(written_bytes);
    } else {
      return ApiResult<size_t>::err(std::strerror(errno));
    }
  }

  /*! \brief Read data from the socket.
   *
   * \param [inout] data A buffer to receive the data to. This vector should
   * have reserved capacity corresponding to the amount of bytes to read.
   *
   * \return An ApiResult indicating the outcome of the operation, on success
   * contains the number of bytes read. Note that \ref data will be resized to
   * the number of bytes read, which might be lower than its initial capacity.
   */
  ApiResult<size_t> recv(std::vector<uint8_t> &data) {
    data.resize(data.capacity());
    auto read_bytes = read(_file_descriptor, data.data(), data.capacity());

    if (read_bytes > 0) {
      data.resize(read_bytes);
      return ApiResult<size_t>::ok(read_bytes);
    } else {
      return ApiResult<size_t>::err(std::strerror(errno));
    }
  }

private:
  SocketConnection(const SocketConnection &) = delete;
  SocketConnection(SocketAddress address, uint32_t file_descriptor)
      : _address(address), _file_descriptor(file_descriptor) {}

  SocketAddress _address;
  int32_t _file_descriptor;
};

/*! \cond INTERNAL_MEMBERS */
template <typename ResultType> class InputImpl {
private:
  static ApiResult<ResultType> get_input(std::unique_ptr<IHostApi> &host_api,
                                         std::string key, bool blocking);
  static ApiResult<ResultType>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking);
};
/*! \endcond */

/*! \brief Get a function input as ResultType, blocking until it is available.
 * \param [in] key The name of the input to get.
 * \return An ApiResult indicating the outcome of the operation, on success
 * contains a single value of ResultType.
 */
template <typename ResultType>
inline ApiResult<ResultType> get_input(const std::string &key) {
  return InputImpl<ResultType>::get_input(g_impl, key, true);
}

/*! \brief Get a function input as ResultType.
 * \param [in] key The name of the input to get.
 * \param [in] blocking Whether the function is allowed to block.
 * \return An ApiResult indicating the outcome of the operation, on success
 * contains a single value of ResultType.
 */
template <typename ResultType>
inline ApiResult<ResultType> get_input(const std::string &key, bool blocking) {
  return InputImpl<ResultType>::get_input(g_impl, key, blocking);
}

/*! \brief Get multiple values from a function input as ResultType, allowing
 * blocking when fetching new data. \param [in] key The name of the input to
 * get. \param [in] fetch_size The amount of inputs to get for each fetch from
 * the host. \return An ApiResult indicating the outcome of the operation, on
 * success contains an InputValues of ResultType.
 */
template <typename ResultType>
inline ApiResult<InputValues<ResultType>>
get_input_values(const std::string key, host::firm_size_t fetch_size) {
  return InputImpl<ResultType>::get_input_values(g_impl, key, fetch_size, true);
}

/*! \brief Get multiple values from a function input as ResultType.
 * \param [in] key The name of the input to get.
 * \param [in] fetch_size The amount of inputs to get for each fetch from the
 * host.
 * \param [in] blocking Whether the function is allowed to block. \return
 * An ApiResult indicating the outcome of the operation, on success contains an
 * InputValues of ResultType.
 */
template <typename ResultType>
inline ApiResult<InputValues<ResultType>>
get_input_values(const std::string key, host::firm_size_t fetch_size,
                 bool blocking) {
  return InputImpl<ResultType>::get_input_values(g_impl, key, fetch_size,
                                                 blocking);
}
/*! \cond INTERNAL_MEMBERS */
/*
  Strings
*/
class StringIterator : public IInputIterator<std::string> {
public:
  StringIterator(host::StringIterator *raw_iter) : _raw_iter(raw_iter) {}

  ~StringIterator() { g_impl->close_string_iterator(_raw_iter); }

private:
  virtual std::pair<const std::string, bool> next_value() override {
    char *result = nullptr;
    auto api_result = g_impl->next_string(_raw_iter, &result);
    if (host::ff_result_is_ok(&api_result)) {
      auto s = std::string(result);
      std::free((void *)result);
      return std::make_pair(s, true);
    }

    // TODO: what to do on error mid-iteration
    return std::make_pair(std::string(), false);
  }

  host::StringIterator *_raw_iter;
};

template <> class InputImpl<std::string> {
public:
  static ApiResult<std::string> get_input(std::unique_ptr<IHostApi> &host_api,
                                          std::string key, bool blocking) {
    char *str = nullptr;
    auto result = host_api->get_string_input(key.c_str(), blocking, &str);

    if (host::ff_result_is_ok(&result)) {
      auto result_str = std::string(str);
      std::free(str);
      return ApiResult<std::string>::ok(result_str);
    } else {
      return ApiResult<std::string>::err(result.error_msg);
    }
  }

  static ApiResult<InputValues<std::string>>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking) {
    host::StringIterator *raw_iter = nullptr;
    auto api_result = host_api->get_string_iterator(key.c_str(), fetch_size,
                                                    blocking, &raw_iter);
    if (host::ff_result_is_ok(&api_result)) {
      std::shared_ptr<IInputIterator<std::string>> string_iter =
          std::make_shared<StringIterator>(StringIterator(raw_iter));
      return ApiResult<InputValues<std::string>>::ok(
          InputValues<std::string>(string_iter));
    } else {
      return ApiResult<InputValues<std::string>>::err(api_result.error_msg);
    }
  }
};

/*
  Ints
*/
class IntIterator : public IInputIterator<host::firm_int_t> {
public:
  IntIterator(host::IntIterator *raw_iter) : _raw_iter(raw_iter) {}

  ~IntIterator() { g_impl->close_int_iterator(_raw_iter); }

private:
  virtual std::pair<const host::firm_int_t, bool> next_value() override {
    host::firm_int_t result;
    auto api_result = g_impl->next_int(_raw_iter, &result);
    if (host::ff_result_is_ok(&api_result)) {
      return std::make_pair(result, true);
    }

    // TODO: what to do on error mid-iteration
    return std::make_pair(0, false);
  }

  host::IntIterator *_raw_iter;
};

template <> class InputImpl<host::firm_int_t> {
public:
  static ApiResult<host::firm_int_t>
  get_input(std::unique_ptr<IHostApi> &host_api, std::string key,
            bool blocking) {
    host::firm_int_t i;
    auto result = host_api->get_int_input(key.c_str(), blocking, &i);

    if (host::ff_result_is_ok(&result)) {
      return ApiResult<host::firm_int_t>::ok(i);
    } else {
      return ApiResult<host::firm_int_t>::err(result.error_msg);
    }
  }

  static ApiResult<InputValues<host::firm_int_t>>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking) {
    host::IntIterator *raw_iter = nullptr;
    auto api_result = host_api->get_int_iterator(key.c_str(), fetch_size,
                                                 blocking, &raw_iter);
    if (host::ff_result_is_ok(&api_result)) {
      std::shared_ptr<IInputIterator<host::firm_int_t>> int_iter =
          std::make_shared<IntIterator>(IntIterator(raw_iter));
      return ApiResult<InputValues<host::firm_int_t>>::ok(
          InputValues<host::firm_int_t>(int_iter));
    } else {
      return ApiResult<InputValues<host::firm_int_t>>::err(
          api_result.error_msg);
    }
  }
};

/*
  Floats
*/
class FloatIterator : public IInputIterator<host::firm_float_t> {
public:
  FloatIterator(host::FloatIterator *raw_iter) : _raw_iter(raw_iter) {}

  ~FloatIterator() { g_impl->close_float_iterator(_raw_iter); }

private:
  virtual std::pair<const host::firm_float_t, bool> next_value() override {
    host::firm_float_t result;
    auto api_result = g_impl->next_float(_raw_iter, &result);
    if (host::ff_result_is_ok(&api_result)) {
      return std::make_pair(result, true);
    }

    // TODO: what to do on error mid-iteration
    return std::make_pair(0, false);
  }

  host::FloatIterator *_raw_iter;
};

template <> class InputImpl<host::firm_float_t> {
public:
  static ApiResult<host::firm_float_t>
  get_input(std::unique_ptr<IHostApi> &host_api, std::string key,
            bool blocking) {
    host::firm_float_t i;
    auto result = host_api->get_float_input(key.c_str(), blocking, &i);

    if (host::ff_result_is_ok(&result)) {
      return ApiResult<host::firm_float_t>::ok(i);
    } else {
      return ApiResult<host::firm_float_t>::err(result.error_msg);
    }
  }

  static ApiResult<InputValues<host::firm_float_t>>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking) {
    host::FloatIterator *raw_iter = nullptr;
    auto api_result = host_api->get_float_iterator(key.c_str(), fetch_size,
                                                   blocking, &raw_iter);
    if (host::ff_result_is_ok(&api_result)) {
      std::shared_ptr<IInputIterator<host::firm_float_t>> float_iter =
          std::make_shared<FloatIterator>(FloatIterator(raw_iter));
      return ApiResult<InputValues<host::firm_float_t>>::ok(
          InputValues<host::firm_float_t>(float_iter));
    } else {
      return ApiResult<InputValues<host::firm_float_t>>::err(
          api_result.error_msg);
    }
  }
};

/*
  Bools
*/
class BoolIterator : public IInputIterator<host::firm_bool_t> {
public:
  BoolIterator(host::BoolIterator *raw_iter) : _raw_iter(raw_iter) {}

  ~BoolIterator() { g_impl->close_bool_iterator(_raw_iter); }

private:
  virtual std::pair<const host::firm_bool_t, bool> next_value() override {
    host::firm_bool_t result;
    auto api_result = g_impl->next_bool(_raw_iter, &result);
    if (host::ff_result_is_ok(&api_result)) {
      return std::make_pair(result, true);
    }

    // TODO: what to do on error mid-iteration
    return std::make_pair(0, false);
  }

  host::BoolIterator *_raw_iter;
};

template <> class InputImpl<host::firm_bool_t> {
public:
  static ApiResult<host::firm_bool_t>
  get_input(std::unique_ptr<IHostApi> &host_api, std::string key,
            bool blocking) {
    host::firm_bool_t i;
    auto result = host_api->get_bool_input(key.c_str(), blocking, &i);

    if (host::ff_result_is_ok(&result)) {
      return ApiResult<host::firm_bool_t>::ok(i);
    } else {
      return ApiResult<host::firm_bool_t>::err(result.error_msg);
    }
  }

  static ApiResult<InputValues<host::firm_bool_t>>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking) {
    host::BoolIterator *raw_iter = nullptr;
    auto api_result = host_api->get_bool_iterator(key.c_str(), fetch_size,
                                                  blocking, &raw_iter);
    if (host::ff_result_is_ok(&api_result)) {
      std::shared_ptr<IInputIterator<host::firm_bool_t>> bool_iter =
          std::make_shared<BoolIterator>(BoolIterator(raw_iter));
      return ApiResult<InputValues<host::firm_bool_t>>::ok(
          InputValues<host::firm_bool_t>(bool_iter));
    } else {
      return ApiResult<InputValues<host::firm_bool_t>>::err(
          api_result.error_msg);
    }
  }
};

/*
  Bytes
*/
class ByteIterator : public IInputIterator<host::firm_byte_t> {
public:
  ByteIterator(host::ByteIterator *raw_iter) : _raw_iter(raw_iter) {}

  ~ByteIterator() { g_impl->close_byte_iterator(_raw_iter); }

private:
  virtual std::pair<const host::firm_byte_t, bool> next_value() override {
    host::firm_byte_t result;
    auto api_result = g_impl->next_byte(_raw_iter, &result);
    if (host::ff_result_is_ok(&api_result)) {
      return std::make_pair(result, true);
    }

    // TODO: what to do on error mid-iteration
    return std::make_pair(0, false);
  }

  host::ByteIterator *_raw_iter;
};

template <> class InputImpl<host::firm_byte_t> {
public:
  static ApiResult<host::firm_byte_t>
  get_input(std::unique_ptr<IHostApi> &host_api, std::string key,
            bool blocking) {
    host::firm_byte_t i;
    auto result = host_api->get_byte_input(key.c_str(), blocking, &i);

    if (host::ff_result_is_ok(&result)) {
      return ApiResult<host::firm_byte_t>::ok(i);
    } else {
      return ApiResult<host::firm_byte_t>::err(result.error_msg);
    }
  }

  static ApiResult<InputValues<host::firm_byte_t>>
  get_input_values(std::unique_ptr<IHostApi> &host_api, std::string key,
                   host::firm_size_t fetch_size, bool blocking) {
    host::ByteIterator *raw_iter = nullptr;
    auto api_result = host_api->get_byte_iterator(key.c_str(), fetch_size,
                                                  blocking, &raw_iter);
    if (host::ff_result_is_ok(&api_result)) {
      std::shared_ptr<IInputIterator<host::firm_byte_t>> byte_iter =
          std::make_shared<ByteIterator>(ByteIterator(raw_iter));
      return ApiResult<InputValues<host::firm_byte_t>>::ok(
          InputValues<host::firm_byte_t>(byte_iter));
    } else {
      return ApiResult<InputValues<host::firm_byte_t>>::err(
          api_result.error_msg);
    }
  }
};
/*! \endcond */
}; // namespace firm

#endif
