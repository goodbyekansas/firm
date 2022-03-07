#include <stdio.h>

#include "test.h"

int main() {
    printf("\n🕴️ \033[1;36mRunning basic tests!\033[0m\n");
    run_common_tests();

    printf("\n🧵 \033[1;36mRunning string tests!\033[0m\n");
    run_string_tests();

    printf("\n🧮 \033[1;36mRunning integer tests!\033[0m\n");
    run_integer_tests();

    printf("\n➿ \033[1;36mRunning float tests!\033[0m\n");
    run_float_tests();

    printf("\n🅱️ \033[1;36mRunning bool tests!\033[0m\n");
    run_bool_tests();

    printf("\n👄 \033[1;36mRunning byte tests!\033[0m\n");
    run_byte_tests();

    printf("\n💂 \033[1;32mall tests succeeded!\033[0m\n");
    return 0;
}
