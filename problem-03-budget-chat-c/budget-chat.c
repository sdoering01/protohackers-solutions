// DISCLAIMER: This only works on macOS and BSD systems, since it uses kqueue.
//
// For Linux, you could use epoll instead.

#include <stdlib.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <sys/errno.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/event.h>
#include <arpa/inet.h>

#define PORT 10000
#define MAX_INCOMING_CONNECTIONS 20
#define MAX_CONNECTIONS 64

#define MAX_EVENTS 10
#define BUFFER_SIZE 1024
#define MAX_MESSAGE_LENGTH 1024

#define MAX_NAME_LENGTH 32

#define WELCOME_MESSAGE "Welcome to budgetchat! How should I call you?\n"
#define INVALID_NAME_MESSAGE "Invalid or duplicate name\n"

int write_all(int fd, const char *data, size_t size) {
    const char *data_ptr = data;

    while (size > 0) {
        ssize_t n = write(fd, data_ptr, size);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        data_ptr += n;
        size -= n;
    }

    return data_ptr - data;
}

typedef struct {
    char *data;
    size_t size;
    size_t capacity;
    size_t no_newline_until;
} line_buffer_t;

line_buffer_t
line_buffer_create() {
    line_buffer_t buffer = {
        .data = NULL,
        .size = 0,
        .capacity = 0,
        .no_newline_until = 0,
    };

    return buffer;
}

void
line_buffer_append(line_buffer_t *buffer, char *data, size_t size) {
    if (size == 0) {
        return;
    }

    if (buffer->capacity == 0) {
        buffer->capacity = 1;
    }

    while (buffer->size + size > buffer->capacity) {
        buffer->capacity *= 2;
    }

    buffer->data = realloc(buffer->data, buffer->capacity);
    memcpy(buffer->data + buffer->size, data, size);
    buffer->size += size;
}

/**
 * Reads a line from the buffer.
 *
 * The line is returned as a null-terminated string and does not include the
 * newline character.
 *
 * If there is no newline character in the buffer, NULL is returned.
 */
char *
line_buffer_read_line(line_buffer_t *buffer) {
    char *line = NULL;
    int found_newline = 0;

    for (; buffer->no_newline_until < buffer->size; buffer->no_newline_until++) {
        if (buffer->data[buffer->no_newline_until] == '\n') {
            found_newline = 1;
            break;
        }
    }

    if (found_newline) {
        size_t line_size = buffer->no_newline_until + 1;
        line = malloc(line_size);
        memcpy(line, buffer->data, line_size);
        line[line_size - 1] = '\0';

        memmove(buffer->data, buffer->data + line_size, buffer->size - line_size);
        buffer->size -= line_size;
        buffer->no_newline_until = 0;
    }

    return line;
}

void
line_buffer_destroy(line_buffer_t *buffer) {
    free(buffer->data);
    buffer->size = 0;
    buffer->capacity = 0;
    buffer->no_newline_until = 0;
}

typedef struct {
    int fd;
    int should_disconnect;
    /**
     * This also serves as a flag to indicate whether the client has entered
     * the room or not. NULL means the client has not entered the room yet.
     */
    char *name;
    line_buffer_t in_buffer;
} client_t;

client_t *
client_create(int fd) {
    client_t *client = malloc(sizeof(client_t));
    client->fd = fd;
    client->should_disconnect = 0;
    client->name = NULL;
    client->in_buffer = line_buffer_create();

    return client;
}

void
client_destroy(client_t *client) {
    free(client->name);
    line_buffer_destroy(&client->in_buffer);
    client->name = NULL;
}

typedef struct {
    int kq;
    client_t **clients;
    int n_clients;
} server_context_t;

ssize_t
add_client(server_context_t *ctx, int client_fd) {
    if (ctx->n_clients >= MAX_CONNECTIONS) {
        return -1;
    }

    size_t client_idx = 0;
    for (; client_idx < MAX_CONNECTIONS; client_idx++) {
        if (!ctx->clients[client_idx]) {
            break;
        }
    }

    ctx->clients[client_idx] = client_create(client_fd);
    ctx->n_clients += 1;

    return client_idx;
}

int
is_valid_name(const server_context_t *ctx, const char *name) {
    size_t len = 0;
    for (; name[len] != '\0'; len++) {
        if (
            !(name[len] >= 'a' && name[len] <= 'z') &&
            !(name[len] >= 'A' && name[len] <= 'Z') &&
            !(name[len] >= '0' && name[len] <= '9')
        ) {
            return 0;
        }
    }

    if (len == 0 || len > MAX_NAME_LENGTH) {
        return 0;
    }

    for (size_t i = 0; i < MAX_CONNECTIONS; i++) {
        if (ctx->clients[i] && ctx->clients[i]->name && strcmp(ctx->clients[i]->name, name) == 0) {
            return 0;
        }
    }

    return 1;
}

char *
get_name_list(const server_context_t *ctx) {
    size_t total_len = 0;
    size_t joined_clients = 0;
    for (size_t i = 0; i < MAX_CONNECTIONS; i++) {
        if (ctx->clients[i] && ctx->clients[i]->name) {
            total_len += strlen(ctx->clients[i]->name);
            joined_clients += 1;
        }
    }

    if (joined_clients > 0) {
        total_len += 2 * (joined_clients - 1);
    }

    char *names = malloc(total_len + 1);
    size_t offset = 0;
    int nth_name = 0;
    for (size_t i = 0; i < MAX_CONNECTIONS; i++) {
        if (ctx->clients[i] && ctx->clients[i]->name) {
            strcpy(names + offset, ctx->clients[i]->name);
            offset += strlen(ctx->clients[i]->name);
            if (nth_name < joined_clients - 1) {
                *(names + offset) = ',';
                *(names + offset + 1) = ' ';
                offset += 2;
            }
            nth_name += 1;
        }
    }
    names[total_len] = '\0';

    return names;
}

void
broadcast(server_context_t *ctx, const char *msg, size_t len, int except_fd) {
    for (size_t i = 0; i < MAX_CONNECTIONS; i++) {
        if (
            ctx->clients[i] &&
            ctx->clients[i]->name &&
            !ctx->clients[i]->should_disconnect &&
            ctx->clients[i]->fd != except_fd
        ) {
            if (write_all(ctx->clients[i]->fd, msg, len) < 0) {
                ctx->clients[i]->should_disconnect = 1;
            }
        }
    }
}

void
disconnect_client_by_fd(server_context_t *ctx, int client_fd) {
    struct kevent change;
    EV_SET(&change, client_fd, EVFILT_READ, EV_DELETE, 0, 0, NULL);
    kevent(ctx->kq, &change, 1, NULL, 0, NULL);
    close(client_fd);

    size_t client_idx = 0;
    for (; client_idx < MAX_CONNECTIONS; client_idx++) {
        if (ctx->clients[client_idx] && ctx->clients[client_idx]->fd == client_fd) {
            break;
        }
    }

    if (client_idx == MAX_CONNECTIONS) {
        return;
    }

    if (ctx->clients[client_idx]->name) {
        char *leave_msg;
        asprintf(&leave_msg, "* %s has left the room\n", ctx->clients[client_idx]->name);
        broadcast(ctx, leave_msg, strlen(leave_msg), client_fd);
        free(leave_msg);
    }

    client_destroy(ctx->clients[client_idx]);
    ctx->clients[client_idx] = NULL;
    ctx->n_clients -= 1;
}

void
setup_server(int *server_fd) {
    if ((*server_fd = socket(PF_INET, SOCK_STREAM, 0)) < 0) {
        perror("creating socket failed");
        exit(EXIT_FAILURE);
    }

    int opt = 1;
    if (setsockopt(*server_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt)) < 0) {
        perror("setting socket options failed");
        close(*server_fd);
        exit(EXIT_FAILURE);
    }

    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_addr.s_addr = INADDR_ANY,
        .sin_port = htons(PORT),
    };
    if (bind(*server_fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("binding socket failed");
        close(*server_fd);
        exit(EXIT_FAILURE);
    }

    if (listen(*server_fd, MAX_INCOMING_CONNECTIONS) < 0) {
        perror("listening on socket failed");
        close(*server_fd);
        exit(EXIT_FAILURE);
    }

    printf("Listening on port %d\n", PORT);
}

void
setup_kqueue(int *kq, int server_fd) {
    if ((*kq = kqueue()) < 0) {
        perror("creating kqueue failed");
        close(server_fd);
        exit(EXIT_FAILURE);
    }

    struct kevent change;
    EV_SET(&change, server_fd, EVFILT_READ, EV_ADD, 0, 0, NULL);
    if (kevent(*kq, &change, 1, NULL, 0, NULL) < 0) {
        perror("adding server socket to kqueue failed");
        close(server_fd);
        exit(EXIT_FAILURE);
    }
}

void
handle_server(server_context_t *ctx, int server_fd) {
    struct kevent change;

    int client_fd = accept(server_fd, NULL, NULL);
    if (client_fd < 0) {
        perror("accepting new connection failed");
        return;
    }

    ssize_t client_idx = add_client(ctx, client_fd);
    // Server is full
    if (client_idx < 0) {
        close(client_fd);
        return;
    }

    EV_SET(&change, client_fd, EVFILT_READ, EV_ADD, 0, 0, (void *)client_idx);
    if (kevent(ctx->kq, &change, 1, NULL, 0, NULL) < 0) {
        perror("adding client socket to kqueue failed");
        exit(EXIT_FAILURE);
    }

    if (write_all(client_fd, WELCOME_MESSAGE, sizeof(WELCOME_MESSAGE) - 1) < 0) {
        disconnect_client_by_fd(ctx, client_fd);
    }
}

void
handle_client(server_context_t *ctx, int client_fd, ssize_t client_idx) {
    char buffer[BUFFER_SIZE], *line;

    ssize_t bytes_read = read(client_fd, buffer, sizeof(buffer));
    if (bytes_read <= 0) {
        disconnect_client_by_fd(ctx, client_fd);
    } else {
        line_buffer_append(&ctx->clients[client_idx]->in_buffer, buffer, bytes_read);
        while (NULL != (line = line_buffer_read_line(&ctx->clients[client_idx]->in_buffer))) {
            if (!ctx->clients[client_idx]->name) {
                if (!is_valid_name(ctx, line)) {
                    write_all(client_fd, INVALID_NAME_MESSAGE, sizeof(INVALID_NAME_MESSAGE) - 1);
                    disconnect_client_by_fd(ctx, client_fd);
                    free(line);
                    break;
                }

                char *join_msg;
                asprintf(&join_msg, "* %s has entered the room\n", line);
                broadcast(ctx, join_msg, strlen(join_msg), client_fd);
                free(join_msg);

                char *current_users_msg;
                char *names = get_name_list(ctx);
                asprintf(&current_users_msg, "* The room contains: %s\n", names);
                free(names);

                int write_ret = write_all(client_fd, current_users_msg, strlen(current_users_msg));
                free(current_users_msg);

                if (write_ret < 0) {
                    disconnect_client_by_fd(ctx, client_fd);
                    free(line);
                    break;
                }

                ctx->clients[client_idx]->name = line;
            } else {
                char *msg;
                asprintf(&msg, "[%s] %s\n", ctx->clients[client_idx]->name, line);
                broadcast(ctx, msg, strlen(msg), client_fd);
                free(msg);
                free(line);
            }
        }
    }
}

int
main(void) {
    int server_fd, kq, n_events, i;
    struct kevent events[MAX_EVENTS];

    setup_server(&server_fd);
    setup_kqueue(&kq, server_fd);

    server_context_t ctx = {
        .kq = kq,
        .clients = calloc(MAX_CONNECTIONS, sizeof(client_t *)),
        .n_clients = 0,
    };

    while (1) {
        n_events = kevent(kq, NULL, 0, events, MAX_EVENTS, NULL);
        if (n_events < 0) {
            if (errno == EINTR) {
                continue;
            }
            perror("waiting for kqueue events failed");
            exit(EXIT_FAILURE);
        }

        for (i = 0; i < n_events; i++) {
            if (events[i].ident == server_fd) {
                handle_server(&ctx, server_fd);
            } else {
                int client_fd = events[i].ident;
                ssize_t client_idx = (ssize_t)events[i].udata;
                handle_client(&ctx, client_fd, client_idx);
            }
        }

        for (size_t i = 0; i < MAX_CONNECTIONS; i++) {
            if (ctx.clients[i] && ctx.clients[i]->should_disconnect) {
                disconnect_client_by_fd(&ctx, ctx.clients[i]->fd);
            }
        }
    }
}
