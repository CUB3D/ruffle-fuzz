// A LD_PRELOAD library that mocks the following functions from glibc:
// fopen
// fwrite
// truncate (if HOOK_TRUNCATE is defined)
// These will be changed so that if a file ending in `PATH_ENDING` is fopen'd, a reference to /dev/null will be returned instead
// any future attempt to fwrite to the file descriptor returned by this fopen, will be redirected to stdout which will then be flushed
// if HOOK_TRUNCATE is defined then attempts to truncate paths ending in `PATH_ENDING` will become NO-OPS
// Note that having open handles to files ending in `PATH_ENDING` at the same time will lead to lost writes, as only the most recent file descriptor
// from fopen will be detected and hooked in fwrite
// Setting the DEBUG macro to true will enable verbose logs

#define _GNU_SOURCE

#include <stdio.h>
#include <string.h>
#include <dlfcn.h>
#include <stdbool.h>


#define DEBUG false
//#define HOOK_TRUNCATE

const char* PATH_ENDING = ".macromedia/Flash_Player/Logs/flashlog.txt";

#define DBG(f) if(DEBUG) {\
    f;\
}

typedef FILE* (*orig_fopen_func_type)(const char *path, const char *mode);
typedef int (*orig_fwrite_func_type)(const void* ptr, size_t size, size_t nmemb, FILE* stream);
#ifdef HOOK_TRUNCATE
typedef int (*orig_truncate_func_type)(const char* pathname, int length);
#endif
//typedef int (*orig_open_func_type)(const char *pathname, int flags, ...);
//typedef int (*orig_xstat_func_type)(int version, const char* pathname, struct stat* statbuf);
//typedef int (*orig_write_func_type)(int fd, const void* buf, size_t length);



/*int open(const char *pathname, int flags, ...)
{
    printf("open(%s)\n", pathname);


    orig_open_func_type orig_func;
    orig_func = (orig_open_func_type)dlsym(RTLD_NEXT, "open");

    // If O_CREAT is used to create a file, the file access mode must be given.
    if (flags & O_CREAT) {
        va_list args;
        va_start(args, flags);
        int mode = va_arg(args, int);
        va_end(args);
        return orig_func(pathname, flags, mode);
    } else {
        return orig_func(pathname, flags);
    }
}
int open64(const char *pathname, int flags, ...)
{
    printf("open(%s)\n", pathname);

    orig_open_func_type orig_func;
    orig_func = (orig_open_func_type)dlsym(RTLD_NEXT, "open64");

    // If O_CREAT is used to create a file, the file access mode must be given.
    if (flags & O_CREAT) {
        va_list args;
        va_start(args, flags);
        int mode = va_arg(args, int);
        va_end(args);
        return orig_func(pathname, flags, mode);
    } else {
        return orig_func(pathname, flags);
    }
}*/
/*
int __xstat64(int version, const char* pathname, struct stat* statbuf) 
{
    printf("stat(%s)\n", pathname);

    if(strcmp(pathname, "./test.swf") == 0) {
        return 0;
    }

    orig_xstat_func_type orig_func;
    orig_func = (orig_xstat_func_type)dlsym(RTLD_NEXT, "__xstat64");

    return orig_func(version, pathname, statbuf);
}*/


orig_fopen_func_type glibc_fopen64;
orig_fwrite_func_type glibc_fwrite;
#ifdef HOOK_TRUNCATE
orig_truncate_func_type glibc_truncate;
#endif

__attribute((constructor))
void init_preload() {
    glibc_fopen64 = (orig_fopen_func_type)dlsym(RTLD_NEXT, "fopen64");
    glibc_fwrite = (orig_fwrite_func_type)dlsym(RTLD_NEXT, "fwrite");
#ifdef HOOK_TRUNCATE
    glibc_truncate = (orig_truncate_func_type)dlsym(RTLD_NEXT, "truncate");
#endif
}

/*
Check if a given string `haystack` ends with the given string `needle`
if both strings are null, returns -1
if `haystack` ends with `needle` returns 1
else returns 0
*/
int endswith(const char* haystack, const char* needle) {
    // Check for incoming nulls
    if (!haystack || !needle) {
        return -1;
    }

    size_t haystack_len = strlen(haystack);
    size_t needle_len = strlen(needle);
    // If the needle is larger than the haystack then we will always fail
    if (needle_len > haystack_len) {
        return 0;
    }

    // Go to end of haystack then back by length of needle, if from there to end is equal to needle then return true
    return strncmp(haystack + haystack_len - needle_len, needle, needle_len) == 0;
}

FILE* fake_log = 0;

FILE* fopen64(const char *pathname, const char* flags)
{
    DBG(fprintf(stderr, "fopen(%s)\n", pathname));

    if(endswith(pathname, PATH_ENDING) == 1) {
       fake_log = glibc_fopen64("/dev/null", flags);
       return fake_log;
    }


    return glibc_fopen64(pathname, flags);
}

#ifdef HOOK_TRUNCATE
int truncate(const char* pathname, __off_t length) 
{
    DBG(fprintf(stderr, "truncate(%s)\n", pathname));

    if(endswith(pathname, PATH_ENDING) == 1) {
        return 0;
    }

    return glibc_truncate(pathname, length);
}
#endif

size_t fwrite(const void* ptr, size_t size, size_t nmemb, FILE* stream) {
    DBG(fprintf(stderr, "fwrite(%s, %ld, %ld, %d)\n", (const char*)ptr, size, nmemb, stream->_fileno));
    
    if(fake_log != 0 && stream->_fileno == fake_log->_fileno) {
        DBG(fprintf(stderr, "Ignoring write to log, printing\n"));
        fprintf(stdout, "%s", (const char*) ptr);
        fflush(stdout);
        return size * nmemb;
    }

    return glibc_fwrite(ptr, size, nmemb, stream);
}

// openat
// newfstatat
// lseek
// write
// close
