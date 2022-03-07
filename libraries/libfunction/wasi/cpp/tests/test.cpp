#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <fstream>
#include <iostream>
#include <sstream>
#include <utility>
#include <vector>

#if !defined(__wasi__)
#include <sys/stat.h>
#include <unistd.h>
#endif

#include "function.hh"

#define run_test(fn)                                                           \
  printf("\n🧜 running \033[1;36m" #fn "\033[0m... ");                       \
  fflush(stdout);                                                              \
  fn();                                                                        \
  printf("\033[32mok!\033[0m\n");

char *create_string_ptr(const char *s) {
  auto mem = static_cast<char *>(std::malloc(sizeof(char) * (strlen(s) + 1)));
  std::strcpy(mem, s);

  return static_cast<char *>(mem);
}

namespace common {
class FakeHostOsApi : public firm::IHostApi {
  virtual host::ApiResult get_host_os(char **result) override {
    *result = create_string_ptr("freebsd");
    return host::ApiResult{
        host::ApiResult_Ok,
        "",
    };
  }
};

class FakeErrorHostOsApi : public firm::IHostApi {
  virtual host::ApiResult get_host_os(char **result) override {
    return host::ApiResult{
        host::ApiResult_Error,
        "💣",
    };
  }
};

void test_get_host_os() {
  firm::set_api_impl(new FakeHostOsApi());
  auto result = firm::get_host_os();
  assert(result.is_ok());
  assert(result.value() == "freebsd");

  firm::set_api_impl(new FakeErrorHostOsApi());
  result = firm::get_host_os();
  assert(result.is_error());
  assert(result.error_message() == "💣");
}

class FakeStartHostProcessApi : public firm::IHostApi {
  virtual host::ApiResult
  start_host_process(const host::StartProcessRequest *request,
                     uint64_t *pid_out, int64_t *exit_code_out) override {

    *pid_out = 6497;
    if (request->wait) {
      *exit_code_out = -4;
    }

    _env_vars.clear();
    for (size_t i = 0; i < request->num_env_vars; ++i) {
      auto var = request->env_vars[i];

      _env_vars[std::string(var.key)] = std::string(var.value);
    }

    return host::ApiResult{
        .kind = host::ApiResult_Ok,
    };
  }

public:
  bool env_has_key(const std::string &key) const {
    return _env_vars.find(key) != _env_vars.end();
  }

  const std::string &get_env(const std::string &key) const {
    return (*_env_vars.find(key)).second;
  }

private:
  std::map<std::string, std::string> _env_vars;
};

class FakeStartHostProcessErrorApi : public firm::IHostApi {
  virtual host::ApiResult
  start_host_process(const host::StartProcessRequest *request,
                     uint64_t *pid_out, int64_t *exit_code_out) override {

    return host::ApiResult{.kind = host::ApiResult_Error,
                           .error_msg = "Jag har boots hemma."};
  }
};

void test_start_host_process() {
  auto api = new FakeStartHostProcessApi();
  firm::set_api_impl(api);

  std::map<std::string, std::string> env_vars{{"mega", "cool"},
                                              {"no", "boots"}};

  auto builder = firm::HostProcessBuilder("run object-orgy")
                     .wait(true)
                     .environment_variables(env_vars);
  auto process = builder.start();

  assert(process.is_ok());
  assert(process.value().exited());
  assert(process.value().pid() == 6497);
  assert(process.value().exit_code() == -4);
  assert(api->env_has_key("mega"));
  assert(api->get_env("mega") == "cool");
  assert(api->env_has_key("no"));
  assert(api->get_env("no") == "boots");

  builder.wait(false);
  process = builder.start();
  assert(process.is_ok());
  assert(!process.value().exited());
}

void test_bad_start_host_process() {
  firm::set_api_impl(new FakeStartHostProcessErrorApi());
  auto process = firm::HostProcessBuilder("run object-orgy").wait(true).start();

  assert(process.is_error());
  auto err = process.error_message();
  assert(err.find("Jag har boots hemma.") != std::string::npos);
}

class FakeSetFunctionErrorApi : public firm::IHostApi {
  virtual host::ApiResult
  set_function_error(const char *error_message) override {
    _error_message = std::string(error_message);

    return host::ApiResult{
        .kind = host::ApiResult_Ok,
    };
  }

public:
  const std::string &error_message() const { return _error_message; }

private:
  std::string _error_message;
};

class FakeSetFunctionErrorBadApi : public firm::IHostApi {
  virtual host::ApiResult
  set_function_error(const char *error_message) override {
    return host::ApiResult{.kind = host::ApiResult_Error,
                           .error_msg = "The error errored"};
  }
};

void test_set_function_error() {
  auto api = new FakeSetFunctionErrorApi();
  firm::set_api_impl(api);
  auto expected = "The function did not work";
  auto res = firm::set_function_error(expected);

  assert(res.is_ok());
  assert(api->error_message() == expected);
}

void test_bad_set_function_error() {
  firm::set_api_impl(new FakeSetFunctionErrorBadApi());
  auto res = firm::set_function_error("Nooo!");

  assert(res.is_error());
  assert(res.error_message().find("The error errored") != std::string::npos);
}

class FakeConnectApi : public firm::IHostApi {
  virtual host::ApiResult connect(const char *address,
                                  int32_t *file_descriptor_out) override {
    std::string str_address = _base_path;
    str_address.append(address + 6);

    *file_descriptor_out =
        open(str_address.c_str(), O_CREAT | O_RDWR,
             0660); // The magic number 6 is the protocol:// offset

    if (*file_descriptor_out < 0) {
      return host::ApiResult{
          .kind = host::ApiResult_Error,
          .error_msg = std::strerror(errno),
      };
    }

    return host::ApiResult{
        .kind = host::ApiResult_Ok,
    };
  }

public:
  FakeConnectApi() {
#if defined(__wasi__)
    _base_path = "/";
#else
    char tmpl[] = "/tmp/libfunction-tests-XXXXXX";
    if (mkdtemp(tmpl) == NULL) {
      _base_path = "/invalid-path/";
    } else {
      _base_path = std::string(tmpl) + "/";
      std::cout << "creating temp test dir in " << _base_path << std::endl;
    }
#endif
  }

  const std::string &base_path() { return _base_path; }

private:
  std::string _base_path;
};

class FakeConnectApiSimple : public firm::IHostApi {
  virtual host::ApiResult connect(const char *address,
                                  int32_t *file_descriptor_out) override {
    *file_descriptor_out = -5;
    _address = address;
    return host::ApiResult{
        .kind = host::ApiResult_Ok,
    };
  }

public:
  const std::string &address() const { return _address; }

private:
  std::string _address;
};

void test_connect_addresses() {
  auto api = new FakeConnectApiSimple();
  firm::set_api_impl(api);
  auto res = firm::SocketConnection::connect(firm::SocketConnection::UDP,
                                             "hostname:1337");
  assert(res.is_ok());
  assert(api->address() == "udp://hostname:1337");
  res = firm::SocketConnection::connect(firm::SocketConnection::UDP,
                                        std::make_pair("ghost.name", 9000));
  assert(res.is_ok());
  assert(api->address() == "udp://ghost.name:9000");

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        "host.name:1337");
  assert(res.is_ok());
  assert(api->address() == "tcp://host.name:1337");
  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        std::make_pair("gh👻ost.name", 9000));
  assert(res.is_ok());
  assert(api->address() == "tcp://gh👻ost.name:9000");

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        "123.345.678.23:5000");
  assert(res.is_ok());
  assert(api->address() == "tcp://123.345.678.23:5000");

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        "123.123.123.123:5000");
  assert(res.is_error());

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        "[123::a23]:5000");
  assert(res.is_error());

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP, "noport");
  assert(res.is_error());

  res = firm::SocketConnection::connect(firm::SocketConnection::TCP,
                                        "[::1]:4353");
  assert(res.is_error());
}

void test_connect_send() {
  auto api = new FakeConnectApi();
  firm::set_api_impl(api);

  auto connection =
      firm::SocketConnection::connect(firm::SocketConnection::TCP, "mega:4567");

  assert(connection.is_ok());
  std::string str_data = "I am hecker man.";
  auto data = std::vector<std::uint8_t>(str_data.begin(), str_data.end());
  auto send_res = connection.value().send(data);
  assert(send_res.is_ok());
  assert(send_res.value() == data.size());

  std::ifstream f(api->base_path() + "mega:4567");
  std::stringstream buffer;
  buffer << f.rdbuf();

  assert(buffer.str() == str_data);
}

void test_connect_recv() {
  auto api = new FakeConnectApi();
  firm::set_api_impl(api);
  auto hostname = "mega_host:9999";
  auto stream = std::ofstream(api->base_path() + hostname);
  auto expected_result = "I go from right to left for some reason.";
  stream << expected_result;
  stream.close();

  auto connection =
      firm::SocketConnection::connect(firm::SocketConnection::TCP, hostname);

  assert(connection.is_ok());
  auto data = std::vector<uint8_t>();
  data.reserve(45);
  auto read_result = connection.value().recv(data);

  assert(read_result.is_ok());
  assert(read_result.value() == data.size());

  std::string str_data(data.begin(), data.end());
  assert(str_data == expected_result);
}

} // namespace common

namespace strings {
class FakeGetStringApi : public firm::IHostApi {
  virtual host::ApiResult get_string_input(const char *key, bool blocking,
                                           char **result) override {
    *result = create_string_ptr("Stränga regler");
    return host::ApiResult{
        host::ApiResult_Ok,
        "",
    };
  }
};

class FakeErrorGetStringApi : public firm::IHostApi {
  virtual host::ApiResult get_string_input(const char *key, bool blocking,
                                           char **result) override {
    return host::ApiResult{
        host::ApiResult_Error,
        "💣",
    };
  }
};

void test_get_single_string() {
  firm::set_api_impl(new FakeGetStringApi());
  auto result = firm::get_input<std::string>("does-not-matter");
  assert(result.is_ok());
  assert(result.value() == "Stränga regler");

  firm::set_api_impl(new FakeErrorGetStringApi());
  result = firm::get_input<std::string>("does-not-matter");
  assert(result.is_error());
  assert(result.error_message() == "💣");
}

class FakeGetStringsApi : public firm::IHostApi {
public:
  FakeGetStringsApi() : _num_strings_left(NUM_STRINGS) {
    // yes, this is ugly but life is too short for snprintf and friends
    for (size_t i = 0; i < NUM_STRINGS; ++i) {
      _strings[i] =
          create_string_ptr((std::string("test-") + std::to_string(i)).c_str());
    }
  }

  ~FakeGetStringsApi() {
    for (size_t i = 0; i < NUM_STRINGS; ++i) {
      std::free(_strings[i]);
    }
  }

  static constexpr std::size_t NUM_STRINGS = 10;

private:
  virtual host::ApiResult
  get_string_iterator(const char *key, uint32_t fetch_size, bool blocking,
                      host::StringIterator **result) override {
    // I can do whatever I want
    *result = (host::StringIterator *)0xdeadbeef;
    return host::ApiResult{
        host::ApiResult_Ok,
        "",
    };
  }

  virtual host::ApiResult next_string(host::StringIterator *iter,
                                      char **result) override {
    if (_num_strings_left == 0) {
      return host::ApiResult{host::ApiResult_EndOfInput, ""};
    }

    *result = create_string_ptr(_strings[NUM_STRINGS - _num_strings_left]);
    --_num_strings_left;

    return host::ApiResult{
        host::ApiResult_Ok,
        "",
    };
  }

  // dont really need to close anything since the returned iterator is bogus
  virtual void close_string_iterator(host::StringIterator *iter) override {}

  char *_strings[NUM_STRINGS];
  std::size_t _num_strings_left;
};

void test_get_strings() {
  firm::set_api_impl(new FakeGetStringsApi());
  auto result = firm::get_input_values<std::string>("does-not-matter-here", 3);
  assert(result.is_ok());
  std::size_t index = 0;
  for (const std::string &v : result.value()) {
    assert(v == std::string("test-") + std::to_string(index));
    ++index;
  }
}
} // namespace strings

namespace ints {
class FakeGetIntApi : public firm::IHostApi {
  virtual host::ApiResult get_int_input(const char *key, bool blocking,
                                        host::firm_int_t *result) override {
    *result = 5318008;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }
};

void test_get_single_int() {
  firm::set_api_impl(new FakeGetIntApi());
  auto result = firm::get_input<host::firm_int_t>("does-not-matter");
  assert(result.is_ok());
  assert(result.value() == 5318008);
}

class FakeGetIntsApi : public firm::IHostApi {
public:
  static constexpr std::size_t NUM_INTS = 10;

  FakeGetIntsApi(host::firm_int_t start) : _num_ints_left(NUM_INTS) {
    for (size_t i = 0; i < NUM_INTS; ++i) {
      _ints[i] = start + i;
    }
  }

private:
  virtual host::ApiResult
  get_int_iterator(const char *key, uint32_t fetch_size, bool blocking,
                   host::IntIterator **result) override {
    // I can do whatever I want
    *result = (host::IntIterator *)0xdeadbeef;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  virtual host::ApiResult next_int(host::IntIterator *iter,
                                   host::firm_int_t *result) override {
    if (_num_ints_left == 0) {
      return host::ApiResult{host::ApiResult_EndOfInput, ""};
    }

    *result = _ints[NUM_INTS - _num_ints_left];
    --_num_ints_left;

    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  // dont really need to close anything since the returned iterator is bogus
  virtual void close_int_iterator(host::IntIterator *iter) override {}

  host::firm_int_t _ints[NUM_INTS];
  std::size_t _num_ints_left;
};

void test_get_ints() {
  host::firm_int_t compare = 420;
  firm::set_api_impl(new FakeGetIntsApi(compare));
  auto result =
      firm::get_input_values<host::firm_int_t>("does-not-matter-here", 3);
  assert(result.is_ok());
  for (const host::firm_int_t &v : result.value()) {
    assert(v == compare);
    ++compare;
  }
}
} // namespace ints

namespace floats {
class FakeGetFloatApi : public firm::IHostApi {
  virtual host::ApiResult get_float_input(const char *key, bool blocking,
                                          host::firm_float_t *result) override {
    *result = 7464.1245;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }
};

#define assert_epsi(value1, value2, epsilon)                                   \
  assert(std::abs(value2 - value1) < epsilon)

void test_get_single_float() {
  firm::set_api_impl(new FakeGetFloatApi());
  auto result = firm::get_input<host::firm_float_t>("does-not-matter");
  assert(result.is_ok());
  assert_epsi(7464.1245, result.value(), 0.001);
}

class FakeGetFloatsApi : public firm::IHostApi {
public:
  static constexpr std::size_t NUM_FLOATS = 10;

  FakeGetFloatsApi(host::firm_float_t start) : _num_floats_left(NUM_FLOATS) {
    for (size_t i = 0; i < NUM_FLOATS; ++i) {
      _floats[i] = start + i;
    }
  }

private:
  virtual host::ApiResult
  get_float_iterator(const char *key, uint32_t fetch_size, bool blocking,
                     host::FloatIterator **result) override {
    // I can do whatever I want
    *result = (host::FloatIterator *)0xdeadbeef;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  virtual host::ApiResult next_float(host::FloatIterator *iter,
                                     host::firm_float_t *result) override {
    if (_num_floats_left == 0) {
      return host::ApiResult{host::ApiResult_EndOfInput, ""};
    }

    *result = _floats[NUM_FLOATS - _num_floats_left];
    --_num_floats_left;

    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  // dont really need to close anything since the returned iterator is bogus
  virtual void close_float_iterator(host::FloatIterator *iter) override {}

  host::firm_float_t _floats[NUM_FLOATS];
  std::size_t _num_floats_left;
};

void test_get_floats() {
  host::firm_float_t compare = 0.002;
  firm::set_api_impl(new FakeGetFloatsApi(compare));
  auto result =
      firm::get_input_values<host::firm_float_t>("does-not-matter-here", 3);
  assert(result.is_ok());
  for (const host::firm_float_t &v : result.value()) {
    assert_epsi(v, compare, 0.0001);
    ++compare;
  }
}
} // namespace floats

namespace bools {

class FakeGetBoolApi : public firm::IHostApi {
  virtual host::ApiResult get_bool_input(const char *key, bool blocking,
                                         host::firm_bool_t *result) override {
    *result = true;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }
};

void test_get_single_bool() {
  firm::set_api_impl(new FakeGetBoolApi());
  auto result = firm::get_input<host::firm_bool_t>("does-not-matter");
  assert(result.is_ok());
  assert(true == result.value());
}

class FakeGetBoolsApi : public firm::IHostApi {
public:
  static constexpr std::size_t NUM_BOOLS = 10;

  FakeGetBoolsApi(host::firm_bool_t start) : _num_bools_left(NUM_BOOLS) {
    for (size_t i = 0; i < NUM_BOOLS; ++i) {
      _bools[i] = !((start + i) % 2 == 0);
    }
  }

private:
  virtual host::ApiResult
  get_bool_iterator(const char *key, uint32_t fetch_size, bool blocking,
                    host::BoolIterator **result) override {
    // I can do whatever I want
    *result = (host::BoolIterator *)0xdeadbeef;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  virtual host::ApiResult next_bool(host::BoolIterator *iter,
                                    host::firm_bool_t *result) override {
    if (_num_bools_left == 0) {
      return host::ApiResult{host::ApiResult_EndOfInput, ""};
    }

    *result = _bools[NUM_BOOLS - _num_bools_left];
    --_num_bools_left;

    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  // dont really need to close anything since the returned iterator is bogus
  virtual void close_bool_iterator(host::BoolIterator *iter) override {}

  host::firm_bool_t _bools[NUM_BOOLS];
  std::size_t _num_bools_left;
};

void test_get_bools() {
  host::firm_bool_t compare = true;
  firm::set_api_impl(new FakeGetBoolsApi(compare));
  auto result =
      firm::get_input_values<host::firm_bool_t>("does-not-matter-here", 3);
  assert(result.is_ok());
  for (const host::firm_bool_t &v : result.value()) {
    assert(v == compare);
    compare = !compare;
  }
}
} // namespace bools

namespace bytes {

class FakeGetByteApi : public firm::IHostApi {
  virtual host::ApiResult get_byte_input(const char *key, bool blocking,
                                         host::firm_byte_t *result) override {
    *result = 0xF0;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }
};

void test_get_single_byte() {
  firm::set_api_impl(new FakeGetByteApi());
  auto result = firm::get_input<host::firm_byte_t>("does-not-matter");
  assert(result.is_ok());
  assert(0xF0 == result.value());
}

class FakeGetBytesApi : public firm::IHostApi {
public:
  static constexpr std::size_t NUM_BYTES = 10;

  FakeGetBytesApi(host::firm_byte_t start) : _num_bytes_left(NUM_BYTES) {
    for (size_t i = 0; i < NUM_BYTES; ++i) {
      _bytes[i] = start + i;
    }
  }

private:
  virtual host::ApiResult
  get_byte_iterator(const char *key, uint32_t fetch_size, bool blocking,
                    host::ByteIterator **result) override {
    // I can do whatever I want
    *result = (host::ByteIterator *)0xdeadbeef;
    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  virtual host::ApiResult next_byte(host::ByteIterator *iter,
                                    host::firm_byte_t *result) override {
    if (_num_bytes_left == 0) {
      return host::ApiResult{host::ApiResult_EndOfInput, ""};
    }

    *result = _bytes[NUM_BYTES - _num_bytes_left];
    --_num_bytes_left;

    return host::ApiResult{
        host::ApiResult_Ok,
    };
  }

  // dont really need to close anything since the returned iterator is bogus
  virtual void close_byte_iterator(host::ByteIterator *iter) override {}

  host::firm_byte_t _bytes[NUM_BYTES];
  std::size_t _num_bytes_left;
};

void test_get_bytes() {
  host::firm_byte_t compare = 0x64;
  firm::set_api_impl(new FakeGetBytesApi(compare));
  auto result =
      firm::get_input_values<host::firm_byte_t>("does-not-matter-here", 3);
  assert(result.is_ok());
  for (const host::firm_byte_t &v : result.value()) {
    assert(v == compare);
    ++compare;
  }
}
} // namespace bytes

#if !defined(__wasi__)
extern "C" {
// need these since they are used in tests
bool ff_result_is_ok(const host::ApiResult *result) {
  return result->kind == host::ApiResult_Ok;
}
}
#endif

int main() {
  run_test(common::test_get_host_os);
  run_test(common::test_start_host_process);
  run_test(common::test_bad_start_host_process);
  run_test(common::test_set_function_error);
  run_test(common::test_bad_set_function_error);
  run_test(common::test_connect_addresses);
  run_test(common::test_connect_send);
  run_test(common::test_connect_recv);
  run_test(strings::test_get_single_string);
  run_test(strings::test_get_strings);
  run_test(ints::test_get_single_int);
  run_test(ints::test_get_ints);
  run_test(floats::test_get_single_float);
  run_test(floats::test_get_floats);
  run_test(bools::test_get_single_bool);
  run_test(bools::test_get_bools);
  run_test(bytes::test_get_single_byte);
  run_test(bytes::test_get_bytes);
  return 0;
}
