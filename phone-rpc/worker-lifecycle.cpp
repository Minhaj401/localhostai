// The upstream RPC protocol stays unchanged; this entry point owns OS lifetime.
#ifdef main
#undef main
#endif

#include <csignal>
#include <cstdio>
#include <chrono>
#include <thread>
#include <vector>
#include <string>
#include <poll.h>
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/resource.h>
#include <unistd.h>

int localhostai_rpc_main(int argc, char ** argv);

static bool send_all(int fd, const char * data, size_t size) {
    while (size > 0) {
        const ssize_t sent = send(fd, data, size, MSG_NOSIGNAL);
        if (sent <= 0) return false;
        data += sent;
        size -= static_cast<size_t>(sent);
    }
    return true;
}

static void relay(int client, int upstream) {
    pollfd sockets[2] = {{client, POLLIN, 0}, {upstream, POLLIN, 0}};
    char buffer[64 * 1024];
    while (poll(sockets, 2, -1) > 0) {
        for (int i = 0; i < 2; ++i) {
            if (!(sockets[i].revents & POLLIN)) continue;
            const int from = sockets[i].fd;
            const int to = sockets[1 - i].fd;
            const ssize_t read_bytes = recv(from, buffer, sizeof(buffer), 0);
            if (read_bytes <= 0 || !send_all(to, buffer, static_cast<size_t>(read_bytes))) return;
        }
        if ((sockets[0].revents | sockets[1].revents) & (POLLERR | POLLHUP | POLLNVAL)) return;
    }
}

static void peer_gate(const std::string & allowed, int external_port, int internal_port) {
    in_addr expected{};
    if (inet_pton(AF_INET, allowed.c_str(), &expected) != 1) {
        std::fprintf(stderr, "Invalid --allow-peer IPv4 address\n");
        _exit(2);
    }
    const int listener = socket(AF_INET, SOCK_STREAM, 0);
    int reuse = 1;
    setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
    sockaddr_in bind_address{};
    bind_address.sin_family = AF_INET;
    bind_address.sin_addr.s_addr = htonl(INADDR_ANY);
    bind_address.sin_port = htons(static_cast<uint16_t>(external_port));
    if (listener < 0 || bind(listener, reinterpret_cast<sockaddr *>(&bind_address), sizeof(bind_address)) != 0 || listen(listener, 4) != 0) {
        std::fprintf(stderr, "Failed to create server socket for authenticated RPC gate\n");
        _exit(3);
    }
    for (;;) {
        sockaddr_in peer{};
        socklen_t length = sizeof(peer);
        const int client = accept(listener, reinterpret_cast<sockaddr *>(&peer), &length);
        if (client < 0) continue;
        if (peer.sin_addr.s_addr != expected.s_addr) {
            close(client);
            continue;
        }
        int upstream = -1;
        for (int attempt = 0; attempt < 200 && upstream < 0; ++attempt) {
            upstream = socket(AF_INET, SOCK_STREAM, 0);
            sockaddr_in destination{};
            destination.sin_family = AF_INET;
            destination.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
            destination.sin_port = htons(static_cast<uint16_t>(internal_port));
            if (connect(upstream, reinterpret_cast<sockaddr *>(&destination), sizeof(destination)) != 0) {
                close(upstream);
                upstream = -1;
                std::this_thread::sleep_for(std::chrono::milliseconds(50));
            }
        }
        if (upstream >= 0) relay(client, upstream);
        if (upstream >= 0) close(upstream);
        close(client);
    }
}

int main(int argc, char ** argv) {
    const pid_t parent = getppid();
    if (parent <= 1) {
        std::fprintf(stderr, "Phone worker lost its owning app process\n");
        return 1;
    }
    // A disconnected client must release its RPC session, not kill the listener.
    std::signal(SIGPIPE, SIG_IGN);
    const struct rlimit no_core = {0, 0};
    setrlimit(RLIMIT_CORE, &no_core);
    std::string allowed_peer;
    int external_port = 0;
    int internal_port = 0;
    for (int i = 1; i < argc; ++i) {
        const std::string argument = argv[i];
        if (argument == "--allow-peer" && i + 1 < argc) allowed_peer = argv[++i];
        else if (argument == "--upstream-port" && i + 1 < argc) internal_port = std::stoi(argv[++i]);
        else if ((argument == "--port" || argument == "-p") && i + 1 < argc) external_port = std::stoi(argv[++i]);
    }
    std::vector<std::string> arguments;
    arguments.reserve(static_cast<size_t>(argc));
    arguments.emplace_back(argv[0]);
    for (int i = 1; i < argc; ++i) {
        const std::string argument = argv[i];
        if (argument == "--allow-peer" && i + 1 < argc) {
            ++i;
        } else if (argument == "--upstream-port" && i + 1 < argc) {
            ++i;
        } else if ((argument == "--port" || argument == "-p") && i + 1 < argc) {
            ++i;
            arguments.push_back(argument);
            arguments.push_back(std::to_string(internal_port > 0 ? internal_port : external_port));
        } else if ((argument == "--host" || argument == "-H") && i + 1 < argc && !allowed_peer.empty()) {
            ++i;
            arguments.push_back(argument);
            arguments.emplace_back("127.0.0.1");
        } else {
            arguments.push_back(argument);
        }
    }
    if (!allowed_peer.empty()) {
        if (external_port <= 0 || internal_port <= 0) {
            std::fprintf(stderr, "Authenticated RPC gate requires --port and --upstream-port\n");
            return 2;
        }
        std::thread(peer_gate, allowed_peer, external_port, internal_port).detach();
    }
    std::vector<char *> forwarded;
    for (std::string & argument : arguments) forwarded.push_back(argument.data());
    // PR_SET_PDEATHSIG tracks the spawning thread, which Dart may retire while
    // the app is still alive. Track the parent process instead.
    std::thread([parent] {
        for (;;) {
            if (getppid() != parent) _exit(0);
            std::this_thread::sleep_for(std::chrono::seconds(1));
        }
    }).detach();
    return localhostai_rpc_main(static_cast<int>(forwarded.size()), forwarded.data());
}
