# io-uring cat

This is a small experiment of using io-uring to create a `cat` program in rust using io_uring for asynchronous file reads on linux using the `io-uring` crate.

For each file, the program submits a vectored read `readv` to the submission queue and then reads and prints the result read from the completion queue.

The code base on the cat with liburing example from https://unixism.net/loti/tutorial/cat_liburing.html and fixes some issues, including removing the file size limit of 2^22 bytes.

Due to io_uring being fairly recent, the code only works on modern kernels (tested on 6.13.3, but should work on 5.6 and above).
