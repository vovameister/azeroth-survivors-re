// The app's entry point: hands the main thread to the Rust game (crates/azeroth-survivors-ios),
// whose winit event loop starts UIApplicationMain itself.

extern int azeroth_survivors_main(void);

int main(int argc, char *argv[]) {
    (void)argc;
    (void)argv;
    return azeroth_survivors_main();
}
