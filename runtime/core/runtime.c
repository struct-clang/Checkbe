#include <gc/gc.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct cb_array {
    int64_t len;
    void *data;
} cb_array;

void checkbe_runtime_init(void) {
    GC_init();
}

char *cb_gc_strdup(const char *input) {
    if (input == NULL) {
        return NULL;
    }

    size_t len = strlen(input) + 1;
    char *copy = (char *)GC_MALLOC_ATOMIC(len);
    if (copy == NULL) {
        return NULL;
    }

    memcpy(copy, input, len);
    return copy;
}

char *cb_string_concat(const char *left, const char *right) {
    const char *a = left ? left : "";
    const char *b = right ? right : "";
    size_t len_a = strlen(a);
    size_t len_b = strlen(b);
    size_t total = len_a + len_b + 1;
    char *out = (char *)GC_MALLOC_ATOMIC(total);
    if (out == NULL) {
        return NULL;
    }
    memcpy(out, a, len_a);
    memcpy(out + len_a, b, len_b);
    out[total - 1] = '\0';
    return out;
}

char *cb_int_to_string(int64_t value) {
    char temp[64];
    int written = snprintf(temp, sizeof(temp), "%lld", (long long)value);
    if (written < 0) {
        return cb_gc_strdup("");
    }
    return cb_gc_strdup(temp);
}

char *cb_float_to_string(double value) {
    char temp[128];
    int written = snprintf(temp, sizeof(temp), "%g", value);
    if (written < 0) {
        return cb_gc_strdup("");
    }
    return cb_gc_strdup(temp);
}

char *cb_bool_to_string(bool value) {
    return cb_gc_strdup(value ? "true" : "false");
}

bool cb_string_eq(const char *left, const char *right) {
    if (left == NULL && right == NULL) {
        return true;
    }
    if (left == NULL || right == NULL) {
        return false;
    }
    return strcmp(left, right) == 0;
}

int64_t cb_to_int(const char *value) {
    if (value == NULL) {
        return 0;
    }
    char *end = NULL;
    long long parsed = strtoll(value, &end, 10);
    if (end == value) {
        return 0;
    }
    return (int64_t)parsed;
}

static cb_array *cb_array_alloc(int64_t len, size_t item_size, int atomic) {
    if (len < 0) {
        len = 0;
    }

    cb_array *array = (cb_array *)GC_MALLOC(sizeof(cb_array));
    array->len = len;

    size_t bytes = (size_t)len * item_size;
    array->data = atomic ? GC_MALLOC_ATOMIC(bytes) : GC_MALLOC(bytes);
    if (array->data != NULL) {
        memset(array->data, 0, bytes);
    }

    return array;
}

static int64_t clamp_index(const cb_array *array, int64_t index) {
    if (array == NULL || array->len == 0) {
        return 0;
    }
    if (index < 0) {
        return 0;
    }
    if (index >= array->len) {
        return array->len - 1;
    }
    return index;
}

cb_array *cb_array_new_i64(int64_t len) {
    return cb_array_alloc(len, sizeof(int64_t), 1);
}

cb_array *cb_array_new_f64(int64_t len) {
    return cb_array_alloc(len, sizeof(double), 1);
}

cb_array *cb_array_new_bool(int64_t len) {
    return cb_array_alloc(len, sizeof(uint8_t), 1);
}

cb_array *cb_array_new_str(int64_t len) {
    return cb_array_alloc(len, sizeof(char *), 0);
}

cb_array *cb_array_new_ptr(int64_t len) {
    return cb_array_alloc(len, sizeof(void *), 0);
}

int64_t cb_array_get_i64(const cb_array *array, int64_t index) {
    if (array == NULL || array->data == NULL) {
        return 0;
    }
    const int64_t *items = (const int64_t *)array->data;
    return items[clamp_index(array, index)];
}

double cb_array_get_f64(const cb_array *array, int64_t index) {
    if (array == NULL || array->data == NULL) {
        return 0.0;
    }
    const double *items = (const double *)array->data;
    return items[clamp_index(array, index)];
}

uint8_t cb_array_get_bool(const cb_array *array, int64_t index) {
    if (array == NULL || array->data == NULL) {
        return 0;
    }
    const uint8_t *items = (const uint8_t *)array->data;
    return items[clamp_index(array, index)];
}

char *cb_array_get_str(const cb_array *array, int64_t index) {
    if (array == NULL || array->data == NULL) {
        return NULL;
    }
    char *const *items = (char *const *)array->data;
    return items[clamp_index(array, index)];
}

void *cb_array_get_ptr(const cb_array *array, int64_t index) {
    if (array == NULL || array->data == NULL) {
        return NULL;
    }
    void *const *items = (void *const *)array->data;
    return items[clamp_index(array, index)];
}

void cb_array_set_i64(cb_array *array, int64_t index, int64_t value) {
    if (array == NULL || array->data == NULL) {
        return;
    }
    int64_t *items = (int64_t *)array->data;
    items[clamp_index(array, index)] = value;
}

void cb_array_set_f64(cb_array *array, int64_t index, double value) {
    if (array == NULL || array->data == NULL) {
        return;
    }
    double *items = (double *)array->data;
    items[clamp_index(array, index)] = value;
}

void cb_array_set_bool(cb_array *array, int64_t index, uint8_t value) {
    if (array == NULL || array->data == NULL) {
        return;
    }
    uint8_t *items = (uint8_t *)array->data;
    items[clamp_index(array, index)] = value;
}

void cb_array_set_str(cb_array *array, int64_t index, char *value) {
    if (array == NULL || array->data == NULL) {
        return;
    }
    char **items = (char **)array->data;
    items[clamp_index(array, index)] = value;
}

void cb_array_set_ptr(cb_array *array, int64_t index, void *value) {
    if (array == NULL || array->data == NULL) {
        return;
    }
    void **items = (void **)array->data;
    items[clamp_index(array, index)] = value;
}

cb_array *cb_build_argv_array(int64_t argc, char **argv) {
    cb_array *array = cb_array_new_str(argc);
    if (array == NULL || array->data == NULL || argv == NULL) {
        return array;
    }

    char **items = (char **)array->data;
    for (int64_t i = 0; i < array->len; ++i) {
        const char *value = argv[i] ? argv[i] : "";
        items[i] = cb_gc_strdup(value);
    }
    return array;
}

char *cb_array_str_to_string(const cb_array *array) {
    if (array == NULL || array->data == NULL) {
        return cb_gc_strdup("[]");
    }

    char *const *items = (char *const *)array->data;
    size_t total = 3; /* '[' + ']' + '\0' */
    if (array->len > 1) {
        total += (size_t)(array->len - 1) * 2; /* ", " between elements */
    }

    for (int64_t i = 0; i < array->len; ++i) {
        const char *value = items[i] ? items[i] : "null";
        total += strlen(value);
    }

    char *out = (char *)GC_MALLOC_ATOMIC(total);
    if (out == NULL) {
        return cb_gc_strdup("[]");
    }

    size_t pos = 0;
    out[pos++] = '[';
    for (int64_t i = 0; i < array->len; ++i) {
        if (i > 0) {
            out[pos++] = ',';
            out[pos++] = ' ';
        }
        const char *value = items[i] ? items[i] : "null";
        size_t len = strlen(value);
        memcpy(out + pos, value, len);
        pos += len;
    }
    out[pos++] = ']';
    out[pos] = '\0';
    return out;
}
