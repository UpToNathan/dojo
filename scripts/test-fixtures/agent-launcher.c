#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv)
{
    puts("pinned");
    const char *fixture = getenv("DOJO_LAUNCHER_FIXTURE");
    puts(fixture ? fixture : "unset");
    for (int i = 1; i < argc; ++i)
        puts(argv[i]);
    return 0;
}
