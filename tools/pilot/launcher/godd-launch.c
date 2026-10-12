/* Start lips.exe with the PC BIOS 8x8 font present at linear address 0xFFA6E.
 *
 * The original engine draws all menu/status text from the BIOS ROM font (F000:FA6E), which
 * Windows 9x maps into every process but Wine leaves zeroed. This launcher creates the game
 * suspended, writes the 1 KB font (biosfont.bin, from `lipstool biosfont`) into the child and
 * resumes it. Build: i686-w64-mingw32-gcc -O2 -o godd-launch.exe godd-launch.c
 *
 * It also answers state probes for tools/pilot: when probe.req appears in the working
 * directory, it reads the player object from the game's memory and writes probe.json.
 * poke.req ("x y z angle region [obj]") teleports the player for scripted tests; the probe
 * then also reports object `obj` (its mover hit counter and flags).
 */
#include <windows.h>
#include <stdio.h>

#define BIOS_FONT_ADDR 0x000FFA6E
#define BIOS_FONT_LEN 1024
/* Loader-filled pointer array of objects (re/formats/XRB-records.md); Objects[0] = player. */
#define OBJECTS_PTR_ARRAY 0x0056774C
#define REGION_RECORD 308

static int watched_obj = -1;

static DWORD rd32(HANDLE p, DWORD addr) {
    DWORD v = 0;
    SIZE_T n;
    ReadProcessMemory(p, (LPCVOID)addr, &v, sizeof v, &n);
    return v;
}

static WORD rd16(HANDLE p, DWORD addr) {
    WORD v = 0;
    SIZE_T n;
    ReadProcessMemory(p, (LPCVOID)addr, &v, sizeof v, &n);
    return v;
}

static void wr32(HANDLE p, DWORD addr, DWORD v) {
    SIZE_T n;
    WriteProcessMemory(p, (LPVOID)addr, &v, sizeof v, &n);
}

static void wr16(HANDLE p, DWORD addr, WORD v) {
    SIZE_T n;
    WriteProcessMemory(p, (LPVOID)addr, &v, sizeof v, &n);
}

/* Teleport: region records are one contiguous array, so the target is found from the
 * current region by index (checked against the record's own index field at +0x08). */
static void answer_poke(HANDLE p) {
    double x, y, z;
    int angle, region, obj = -1;
    FILE *f = fopen("poke.req", "r");
    if (!f) return;
    int n = fscanf(f, "%lf %lf %lf %d %d %d", &x, &y, &z, &angle, &region, &obj);
    fclose(f);
    DeleteFileA("poke.req");
    if (n >= 6) watched_obj = obj;
    if (n < 5) return;
    DWORD player = rd32(p, rd32(p, OBJECTS_PTR_ARRAY));
    DWORD point = player ? rd32(p, player + 0x14) : 0;
    DWORD cur = player ? rd32(p, player + 0xFC) : 0;
    if (!point || !cur) return;
    DWORD target = cur + (region - (int)rd16(p, cur + 0x08)) * REGION_RECORD;
    if ((int)rd16(p, target + 0x08) != region) {
        fprintf(stderr, "godd-launch: region %d not found\n", region);
        return;
    }
    wr32(p, point + 0x14, (DWORD)(int)(x * 65536.0));
    wr32(p, point + 0x18, (DWORD)(int)(y * 65536.0));
    wr32(p, player + 0x2C, (DWORD)(int)(z * 65536.0));
    wr16(p, player + 0x24, (WORD)(angle & 0x7FF));
    wr32(p, player + 0xFC, target);
}

static void answer_probe(HANDLE p) {
    DWORD player = rd32(p, rd32(p, OBJECTS_PTR_ARRAY));
    DWORD point = player ? rd32(p, player + 0x14) : 0;
    DWORD region = player ? rd32(p, player + 0xFC) : 0;
    FILE *f = fopen("probe.tmp", "w");
    if (!f) return;
    if (!point) {
        fprintf(f, "{\"error\": \"no player\"}\n");
    } else {
        DWORD state = rd32(p, player + 0x1B4);
        fprintf(f, "{\"x\": %.4f, \"y\": %.4f, \"z\": %.4f, \"angle\": %u, \"region\": %d, \"kills\": %d",
                (int)rd32(p, point + 0x14) / 65536.0, (int)rd32(p, point + 0x18) / 65536.0,
                (int)rd32(p, player + 0x2C) / 65536.0, rd16(p, player + 0x24) & 0x7FF,
                region ? (int)rd16(p, region + 0x08) : -1, state ? (int)rd32(p, state + 0x1C8) : -1);
        if (watched_obj >= 0) {
            DWORD obj = rd32(p, rd32(p, OBJECTS_PTR_ARRAY) + 4 * watched_obj);
            DWORD os = obj ? rd32(p, obj + 0x1B4) : 0;
            /* mover state (re/engine/movers.md): +0x1B0 hit counter, +0x04 event bits */
            fprintf(f, ", \"obj\": {\"hits_left\": %d, \"events\": %d}",
                    os ? (int)rd32(p, os + 0x1B0) : -99,
                    os ? (int)rd32(p, os + 0x04) : -99);
        }
        fprintf(f, "}\n");
    }
    fclose(f);
    DeleteFileA("probe.json");
    MoveFileA("probe.tmp", "probe.json");
}

int main(int argc, char **argv) {
    const char *exe = argc > 1 ? argv[1] : "lips.exe";
    unsigned char font[BIOS_FONT_LEN];
    FILE *f = fopen("biosfont.bin", "rb");
    if (!f || fread(font, 1, sizeof font, f) != sizeof font) {
        fprintf(stderr, "godd-launch: cannot read biosfont.bin\n");
        return 1;
    }
    fclose(f);

    STARTUPINFOA si = {sizeof si};
    PROCESS_INFORMATION pi;
    if (!CreateProcessA(exe, NULL, NULL, NULL, FALSE, CREATE_SUSPENDED, NULL, NULL, &si, &pi)) {
        fprintf(stderr, "godd-launch: CreateProcess(%s) failed: %lu\n", exe, GetLastError());
        return 1;
    }
    LPVOID page = (LPVOID)(BIOS_FONT_ADDR & ~0xFFFu);
    SIZE_T span = (BIOS_FONT_ADDR + BIOS_FONT_LEN) - (SIZE_T)page;
    DWORD old;
    if (!VirtualProtectEx(pi.hProcess, page, span, PAGE_READWRITE, &old)) {
        /* Not mapped yet: reserve and commit the page ourselves. */
        if (!VirtualAllocEx(pi.hProcess, page, span, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE))
            fprintf(stderr, "godd-launch: cannot map BIOS area: %lu\n", GetLastError());
    }
    SIZE_T written = 0;
    if (!WriteProcessMemory(pi.hProcess, (LPVOID)BIOS_FONT_ADDR, font, sizeof font, &written))
        fprintf(stderr, "godd-launch: WriteProcessMemory failed: %lu\n", GetLastError());
    ResumeThread(pi.hThread);
    while (WaitForSingleObject(pi.hProcess, 100) == WAIT_TIMEOUT) {
        if (GetFileAttributesA("probe.req") != INVALID_FILE_ATTRIBUTES) {
            DeleteFileA("probe.req");
            answer_probe(pi.hProcess);
        }
        if (GetFileAttributesA("poke.req") != INVALID_FILE_ATTRIBUTES)
            answer_poke(pi.hProcess);
    }
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    return (int)code;
}
