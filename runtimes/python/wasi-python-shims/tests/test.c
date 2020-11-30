#include "wasi_python_shims.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

#define run_test(fn) \
        printf("\n🧜 running \033[1;36m" #fn "\033[0m... "); \
        fflush(stdout); \
        fn(); \
        printf("\033[32mok!\033[0m\n");

void free_pw(passwd *pw) {
  free(pw->pw_name);
  free(pw->pw_dir);
  free(pw->pw_shell);
  free(pw);
}

void test_chmod() { assert(-1 == chmod("sunes sås.", 1)); }

void test_dup() { assert(-1 == dup(1)); }

void test_umask() {
  assert(0 == umask(0));
  assert(0 == umask(1));
  assert(0 == umask(2));
}

void test_getpwnam() {
  const char *name = "sune";
  passwd *pw = getpwnam(name);
  assert(strcmp(pw->pw_name, name) == 0);
  free_pw(pw);
}

void test_getpwuid() {
  passwd *pw = getpwuid(1);
  assert(pw != NULL);
  free_pw(pw);
}

void test_getpwnam_r() {
  const char *name = "rune";
  passwd p, *pp;
  const size_t bufsize = 512;
  char buff[bufsize];
  int res = getpwnam_r(name, &p, buff, bufsize, &pp);

  assert(res == 0);
  assert(pp == &p);
  assert(strcmp(p.pw_name, name) == 0);
  assert(strlen(p.pw_dir) > 0);
  assert(strlen(p.pw_shell) > 0);
}

void test_getpwuid_r() {
  int id = 5;
  passwd p, *pp;
  const size_t bufsize = 512;
  char buff[bufsize];
  int res = getpwuid_r(id, &p, buff, bufsize, &pp);

  assert(res == 0);
  assert(pp == &p);
  assert(strlen(p.pw_name) > 0);
  assert(strlen(p.pw_dir) > 0);
  assert(strlen(p.pw_shell) > 0);
}

void test_pwent() {
  setpwent();
  passwd *res1 = getpwent();
  passwd *res2 = getpwent();
  endpwent();
  setpwent();
  passwd *res3 = getpwent();
  endpwent();

  assert(res1 != NULL);
  assert(res2 == NULL);
  assert(res3 != NULL);

  free_pw(res1);
  free_pw(res3);
}

void test_getegid() { assert(1 == getegid()); }

void test_geteuid() { assert(1 == geteuid()); }

void test_getgid() { assert(1 == getgid()); }

void test_getuid() { assert(1 == getuid()); }


int main() {
  run_test(test_chmod);
  run_test(test_dup);
  run_test(test_umask);
  run_test(test_getpwnam);
  run_test(test_getpwuid);
  run_test(test_getpwnam_r);
  run_test(test_getpwuid_r);
  run_test(test_pwent);
  run_test(test_getegid);
  run_test(test_geteuid);
  run_test(test_getgid);
  run_test(test_getuid);

  printf("\n💂 \033[1;32mall tests succeeded!\033[0m\n");
  return 0;
}

