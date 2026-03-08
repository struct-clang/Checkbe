#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

char *cb_gc_strdup(const char *input);

void bridge_println_int(long long value) {
    printf("%lld\n", value);
}

void bridge_print_nop(void) {}

void bridge_print_int(long long value) {
    printf("%lld", value);
}

void bridge_println_float(double value) {
    printf("%g\n", value);
}

void bridge_print_float(double value) {
    printf("%g", value);
}

void bridge_println_string(const char *value) {
    if (value == NULL) {
        printf("null\n");
        return;
    }
    printf("%s\n", value);
}

void bridge_print_string(const char *value) {
    if (value == NULL) {
        printf("null");
        return;
    }
    printf("%s", value);
}

void bridge_println3_string(const char *a, const char *b, const char *c) {
    printf("%s%s%s\n", a ? a : "null", b ? b : "null", c ? c : "null");
}

void bridge_println_bool(bool value) {
    printf("%s\n", value ? "true" : "false");
}

void bridge_print_bool(bool value) {
    printf("%s", value ? "true" : "false");
}

void bridge_print_newline(void) {
    printf("\n");
}

char *bridge_readln(void) {
    char *line = NULL;
    size_t size = 0;
    ssize_t read = getline(&line, &size, stdin);
    if (read < 0) {
        if (line != NULL) {
            free(line);
        }
        return cb_gc_strdup("");
    }

    if (read > 0 && line[read - 1] == '\n') {
        line[read - 1] = '\0';
    }

    char *result = cb_gc_strdup(line);
    free(line);
    return result;
}
