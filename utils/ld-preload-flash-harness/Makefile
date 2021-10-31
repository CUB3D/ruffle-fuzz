CFLAGS += -std=c99 -Wall

log_file_accesses.so:
	gcc $(CFLAGS) -shared -fPIC -O2 path-mapping.c -o path-mapping.so -ldl

clean:
	rm *.so
