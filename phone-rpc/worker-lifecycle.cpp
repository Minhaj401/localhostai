// The upstream RPC protocol stays unchanged; this entry point owns OS lifetime.
#ifdef main
#undef main
#endif

#include <csignal>
#include <cstdio>
#include <chrono>
#include <thread>
#include <sys/resource.h>
#include <unistd.h>

int privateml_rpc_main(int argc, char ** argv);

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
    // PR_SET_PDEATHSIG tracks the spawning thread, which Dart may retire while
    // the app is still alive. Track the parent process instead.
    std::thread([parent] {
        for (;;) {
            if (getppid() != parent) _exit(0);
            std::this_thread::sleep_for(std::chrono::seconds(1));
        }
    }).detach();
    return privateml_rpc_main(argc, argv);
}
