#include <ctype.h>
#include <netdb.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/types.h>
#include <unistd.h>

char *cb_gc_strdup(const char *input);

typedef struct {
    char *host;
    char *port;
    char *path;
} ParsedUrl;

typedef struct {
    long long status_code;
    char *body;
} HttpResponse;

static char *gc_dup_or_empty(const char *input) {
    if (input == NULL) {
        return cb_gc_strdup("");
    }
    return cb_gc_strdup(input);
}

static char *heap_dup_range(const char *start, size_t len) {
    char *out = (char *)malloc(len + 1);
    if (out == NULL) {
        return NULL;
    }
    if (len > 0) {
        memcpy(out, start, len);
    }
    out[len] = '\0';
    return out;
}

static char *heap_strdup(const char *input) {
    if (input == NULL) {
        return heap_dup_range("", 0);
    }
    return heap_dup_range(input, strlen(input));
}

static char *normalize_method(const char *method) {
    if (method == NULL || method[0] == '\0') {
        return heap_strdup("GET");
    }

    size_t len = strlen(method);
    char *out = (char *)malloc(len + 1);
    if (out == NULL) {
        return NULL;
    }

    size_t write = 0;
    for (size_t i = 0; i < len; ++i) {
        unsigned char ch = (unsigned char)method[i];
        if (isalpha(ch)) {
            out[write++] = (char)toupper(ch);
        }
    }

    if (write == 0) {
        free(out);
        return heap_strdup("GET");
    }

    out[write] = '\0';
    return out;
}

static bool parse_port(const char *text) {
    if (text == NULL || text[0] == '\0') {
        return false;
    }
    long long value = 0;
    for (const char *ptr = text; *ptr != '\0'; ++ptr) {
        if (!isdigit((unsigned char)*ptr)) {
            return false;
        }
        value = value * 10 + (*ptr - '0');
        if (value > 65535) {
            return false;
        }
    }
    return value > 0;
}

static void free_parsed_url(ParsedUrl *url) {
    if (url == NULL) {
        return;
    }
    free(url->host);
    free(url->port);
    free(url->path);
    url->host = NULL;
    url->port = NULL;
    url->path = NULL;
}

static bool parse_http_url(const char *url, ParsedUrl *out) {
    if (out == NULL) {
        return false;
    }

    out->host = NULL;
    out->port = NULL;
    out->path = NULL;

    if (url == NULL) {
        return false;
    }

    const char *prefix = "http://";
    const size_t prefix_len = strlen(prefix);
    if (strncmp(url, prefix, prefix_len) != 0) {
        return false;
    }

    const char *cursor = url + prefix_len;
    if (*cursor == '\0') {
        return false;
    }

    const char *authority_start = cursor;
    while (*cursor != '\0' && *cursor != '/' && *cursor != '?' && *cursor != '#') {
        cursor++;
    }
    const char *authority_end = cursor;
    if (authority_end == authority_start) {
        return false;
    }

    const char *path_start = cursor;
    if (*path_start == '\0') {
        out->path = heap_strdup("/");
    } else if (*path_start == '?' || *path_start == '#') {
        size_t tail_len = strlen(path_start);
        out->path = (char *)malloc(tail_len + 2);
        if (out->path != NULL) {
            out->path[0] = '/';
            memcpy(out->path + 1, path_start, tail_len + 1);
        }
    } else {
        out->path = heap_strdup(path_start);
    }
    if (out->path == NULL) {
        free_parsed_url(out);
        return false;
    }

    if (*authority_start == '[') {
        const char *closing = memchr(authority_start, ']', (size_t)(authority_end - authority_start));
        if (closing == NULL || closing == authority_start + 1) {
            free_parsed_url(out);
            return false;
        }

        out->host = heap_dup_range(authority_start + 1, (size_t)(closing - authority_start - 1));
        if (out->host == NULL) {
            free_parsed_url(out);
            return false;
        }

        if (closing + 1 < authority_end) {
            if (*(closing + 1) != ':') {
                free_parsed_url(out);
                return false;
            }
            out->port = heap_dup_range(closing + 2, (size_t)(authority_end - (closing + 2)));
        } else {
            out->port = heap_strdup("80");
        }
    } else {
        const char *colon = memchr(authority_start, ':', (size_t)(authority_end - authority_start));
        if (colon != NULL) {
            out->host = heap_dup_range(authority_start, (size_t)(colon - authority_start));
            out->port = heap_dup_range(colon + 1, (size_t)(authority_end - (colon + 1)));
        } else {
            out->host = heap_dup_range(authority_start, (size_t)(authority_end - authority_start));
            out->port = heap_strdup("80");
        }
    }

    if (out->host == NULL || out->port == NULL || out->host[0] == '\0' || !parse_port(out->port)) {
        free_parsed_url(out);
        return false;
    }

    return true;
}

static void set_socket_timeouts(int fd, int seconds) {
    struct timeval timeout;
    timeout.tv_sec = seconds;
    timeout.tv_usec = 0;
    (void)setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    (void)setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
}

static int connect_tcp(const char *host, const char *port) {
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;

    struct addrinfo *result = NULL;
    if (getaddrinfo(host, port, &hints, &result) != 0 || result == NULL) {
        return -1;
    }

    int fd = -1;
    for (struct addrinfo *entry = result; entry != NULL; entry = entry->ai_next) {
        fd = socket(entry->ai_family, entry->ai_socktype, entry->ai_protocol);
        if (fd < 0) {
            continue;
        }

        set_socket_timeouts(fd, 15);
        if (connect(fd, entry->ai_addr, entry->ai_addrlen) == 0) {
            break;
        }

        close(fd);
        fd = -1;
    }

    freeaddrinfo(result);
    return fd;
}

static bool send_all(int fd, const char *data, size_t size) {
    size_t sent = 0;
    while (sent < size) {
        ssize_t written = send(fd, data + sent, size - sent, 0);
        if (written <= 0) {
            return false;
        }
        sent += (size_t)written;
    }
    return true;
}

static bool read_all(int fd, char **out, size_t *out_len) {
    *out = NULL;
    *out_len = 0;

    size_t cap = 8192;
    size_t len = 0;
    char *buffer = (char *)malloc(cap + 1);
    if (buffer == NULL) {
        return false;
    }

    for (;;) {
        if (len == cap) {
            size_t next_cap = cap * 2;
            char *next = (char *)realloc(buffer, next_cap + 1);
            if (next == NULL) {
                free(buffer);
                return false;
            }
            buffer = next;
            cap = next_cap;
        }

        ssize_t read_count = recv(fd, buffer + len, cap - len, 0);
        if (read_count == 0) {
            break;
        }
        if (read_count < 0) {
            free(buffer);
            return false;
        }
        len += (size_t)read_count;
    }

    buffer[len] = '\0';
    *out = buffer;
    *out_len = len;
    return true;
}

static size_t find_header_end(const char *text, size_t len) {
    for (size_t i = 0; i + 3 < len; ++i) {
        if (text[i] == '\r' && text[i + 1] == '\n' && text[i + 2] == '\r' && text[i + 3] == '\n') {
            return i + 4;
        }
    }
    for (size_t i = 0; i + 1 < len; ++i) {
        if (text[i] == '\n' && text[i + 1] == '\n') {
            return i + 2;
        }
    }
    return 0;
}

static long long parse_status_code(const char *response, size_t header_len) {
    if (response == NULL || header_len == 0) {
        return 0;
    }

    const char *line_end = memchr(response, '\n', header_len);
    size_t first_line_len = line_end != NULL ? (size_t)(line_end - response) : header_len;
    if (first_line_len == 0) {
        return 0;
    }

    char first_line[256];
    if (first_line_len >= sizeof(first_line)) {
        first_line_len = sizeof(first_line) - 1;
    }
    memcpy(first_line, response, first_line_len);
    first_line[first_line_len] = '\0';

    char *space = strchr(first_line, ' ');
    if (space == NULL) {
        return 0;
    }
    while (*space == ' ') {
        space++;
    }

    if (!isdigit((unsigned char)*space)) {
        return 0;
    }

    return strtoll(space, NULL, 10);
}

static bool ascii_case_contains(const char *haystack, size_t hay_len, const char *needle) {
    size_t needle_len = strlen(needle);
    if (needle_len == 0 || hay_len < needle_len) {
        return false;
    }

    for (size_t i = 0; i + needle_len <= hay_len; ++i) {
        bool match = true;
        for (size_t j = 0; j < needle_len; ++j) {
            unsigned char a = (unsigned char)haystack[i + j];
            unsigned char b = (unsigned char)needle[j];
            if (tolower(a) != tolower(b)) {
                match = false;
                break;
            }
        }
        if (match) {
            return true;
        }
    }

    return false;
}

static bool has_chunked_transfer(const char *headers, size_t header_len) {
    return ascii_case_contains(headers, header_len, "transfer-encoding: chunked");
}

static bool decode_chunked_body(const char *chunked, size_t chunked_len, char **out_body) {
    *out_body = NULL;

    size_t cap = chunked_len + 1;
    char *buffer = (char *)malloc(cap);
    if (buffer == NULL) {
        return false;
    }

    size_t write = 0;
    size_t pos = 0;
    while (pos < chunked_len) {
        size_t line_start = pos;
        while (pos < chunked_len && chunked[pos] != '\n') {
            pos++;
        }
        if (pos >= chunked_len) {
            free(buffer);
            return false;
        }

        size_t line_end = pos;
        if (line_end > line_start && chunked[line_end - 1] == '\r') {
            line_end--;
        }
        pos++;

        size_t hex_end = line_start;
        while (hex_end < line_end && chunked[hex_end] != ';' && chunked[hex_end] != ' ') {
            hex_end++;
        }
        if (hex_end == line_start) {
            free(buffer);
            return false;
        }

        char hex[32];
        size_t hex_len = hex_end - line_start;
        if (hex_len >= sizeof(hex)) {
            free(buffer);
            return false;
        }
        memcpy(hex, chunked + line_start, hex_len);
        hex[hex_len] = '\0';

        unsigned long long chunk_size = strtoull(hex, NULL, 16);
        if (chunk_size == 0) {
            buffer[write] = '\0';
            *out_body = buffer;
            return true;
        }

        if (chunk_size > (unsigned long long)(chunked_len - pos)) {
            free(buffer);
            return false;
        }

        if (write + (size_t)chunk_size + 1 > cap) {
            size_t next_cap = cap;
            while (write + (size_t)chunk_size + 1 > next_cap) {
                next_cap *= 2;
            }
            char *next = (char *)realloc(buffer, next_cap);
            if (next == NULL) {
                free(buffer);
                return false;
            }
            buffer = next;
            cap = next_cap;
        }

        memcpy(buffer + write, chunked + pos, (size_t)chunk_size);
        write += (size_t)chunk_size;
        pos += (size_t)chunk_size;

        if (pos < chunked_len && chunked[pos] == '\r') {
            pos++;
        }
        if (pos < chunked_len && chunked[pos] == '\n') {
            pos++;
        }
    }

    free(buffer);
    return false;
}

static bool parse_http_response(char *raw, size_t raw_len, HttpResponse *out) {
    out->status_code = 0;
    out->body = heap_strdup("");
    if (out->body == NULL) {
        return false;
    }

    if (raw == NULL || raw_len == 0) {
        return true;
    }

    size_t header_end = find_header_end(raw, raw_len);
    if (header_end == 0) {
        free(out->body);
        out->body = heap_dup_range(raw, raw_len);
        return out->body != NULL;
    }

    out->status_code = parse_status_code(raw, header_end);
    const char *body_start = raw + header_end;
    size_t body_len = raw_len - header_end;

    char *parsed_body = NULL;
    if (has_chunked_transfer(raw, header_end)) {
        if (!decode_chunked_body(body_start, body_len, &parsed_body)) {
            parsed_body = heap_dup_range(body_start, body_len);
        }
    } else {
        parsed_body = heap_dup_range(body_start, body_len);
    }

    if (parsed_body == NULL) {
        return false;
    }

    free(out->body);
    out->body = parsed_body;
    return true;
}

static bool http_request_internal(
    const char *method,
    const char *url,
    const char *body,
    HttpResponse *out
) {
    out->status_code = 0;
    out->body = heap_strdup("");
    if (out->body == NULL) {
        return false;
    }

    ParsedUrl parsed;
    if (!parse_http_url(url, &parsed)) {
        return true;
    }

    char *normalized_method = normalize_method(method);
    if (normalized_method == NULL) {
        free_parsed_url(&parsed);
        return false;
    }

    const char *payload = body != NULL ? body : "";
    size_t payload_len = strlen(payload);
    const bool has_payload = payload_len > 0;

    int request_len = 0;
    if (has_payload) {
        request_len = snprintf(
            NULL,
            0,
            "%s %s HTTP/1.1\r\n"
            "Host: %s\r\n"
            "User-Agent: checkbe-network/1.0\r\n"
            "Connection: close\r\n"
            "Content-Type: application/x-www-form-urlencoded\r\n"
            "Content-Length: %zu\r\n"
            "\r\n"
            "%s",
            normalized_method,
            parsed.path,
            parsed.host,
            payload_len,
            payload
        );
    } else {
        request_len = snprintf(
            NULL,
            0,
            "%s %s HTTP/1.1\r\n"
            "Host: %s\r\n"
            "User-Agent: checkbe-network/1.0\r\n"
            "Connection: close\r\n"
            "\r\n",
            normalized_method,
            parsed.path,
            parsed.host
        );
    }

    if (request_len <= 0) {
        free(normalized_method);
        free_parsed_url(&parsed);
        return false;
    }

    char *request = (char *)malloc((size_t)request_len + 1);
    if (request == NULL) {
        free(normalized_method);
        free_parsed_url(&parsed);
        return false;
    }

    if (has_payload) {
        snprintf(
            request,
            (size_t)request_len + 1,
            "%s %s HTTP/1.1\r\n"
            "Host: %s\r\n"
            "User-Agent: checkbe-network/1.0\r\n"
            "Connection: close\r\n"
            "Content-Type: application/x-www-form-urlencoded\r\n"
            "Content-Length: %zu\r\n"
            "\r\n"
            "%s",
            normalized_method,
            parsed.path,
            parsed.host,
            payload_len,
            payload
        );
    } else {
        snprintf(
            request,
            (size_t)request_len + 1,
            "%s %s HTTP/1.1\r\n"
            "Host: %s\r\n"
            "User-Agent: checkbe-network/1.0\r\n"
            "Connection: close\r\n"
            "\r\n",
            normalized_method,
            parsed.path,
            parsed.host
        );
    }

    int fd = connect_tcp(parsed.host, parsed.port);
    if (fd < 0) {
        free(request);
        free(normalized_method);
        free_parsed_url(&parsed);
        return true;
    }

    bool ok = send_all(fd, request, (size_t)request_len);
    free(request);
    free(normalized_method);
    free_parsed_url(&parsed);

    if (!ok) {
        close(fd);
        return true;
    }

    char *raw_response = NULL;
    size_t raw_len = 0;
    if (!read_all(fd, &raw_response, &raw_len)) {
        close(fd);
        return true;
    }

    close(fd);

    HttpResponse parsed_response;
    if (!parse_http_response(raw_response, raw_len, &parsed_response)) {
        free(raw_response);
        return false;
    }
    free(raw_response);

    free(out->body);
    out->status_code = parsed_response.status_code;
    out->body = parsed_response.body;
    return true;
}

char *network_get(const char *url) {
    HttpResponse response;
    if (!http_request_internal("GET", url, "", &response)) {
        return gc_dup_or_empty("");
    }

    char *result = gc_dup_or_empty(response.body);
    free(response.body);
    return result;
}

char *network_request(const char *method, const char *url, const char *body) {
    HttpResponse response;
    if (!http_request_internal(method, url, body, &response)) {
        return gc_dup_or_empty("");
    }

    char *result = gc_dup_or_empty(response.body);
    free(response.body);
    return result;
}

char *network_post(const char *url, const char *body) {
    return network_request("POST", url, body);
}

long long network_status(const char *url) {
    HttpResponse response;
    if (!http_request_internal("GET", url, "", &response)) {
        return 0;
    }
    long long status = response.status_code;
    free(response.body);
    return status;
}

char *network_resolve(const char *host) {
    if (host == NULL || host[0] == '\0') {
        return gc_dup_or_empty("");
    }

    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;

    struct addrinfo *result = NULL;
    if (getaddrinfo(host, NULL, &hints, &result) != 0 || result == NULL) {
        return gc_dup_or_empty("");
    }

    char host_buffer[NI_MAXHOST];
    char *resolved = gc_dup_or_empty("");

    for (struct addrinfo *entry = result; entry != NULL; entry = entry->ai_next) {
        int rc = getnameinfo(
            entry->ai_addr,
            entry->ai_addrlen,
            host_buffer,
            sizeof(host_buffer),
            NULL,
            0,
            NI_NUMERICHOST
        );
        if (rc == 0) {
            resolved = gc_dup_or_empty(host_buffer);
            break;
        }
    }

    freeaddrinfo(result);
    return resolved;
}

char *network_hostname(void) {
    char host[256];
    if (gethostname(host, sizeof(host)) != 0) {
        return gc_dup_or_empty("");
    }
    host[sizeof(host) - 1] = '\0';
    return gc_dup_or_empty(host);
}
