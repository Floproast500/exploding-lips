"""Enumerate every section / key / script command used by the shipped Exploding Lips INI files.

usage: python3 -I re/scripts/ini_commands.py [GAME_DIR]      (default: ./original)

Reads *.INI / *.ini in GAME_DIR plus the copies inside LIPSINI.ZIP (read in memory, nothing
is extracted).  Command sections are normalised the way lips.exe does it before parsing
(see re/engine/script.md, "Pre-processing"):
  * ';' starts a comment that runs to end of line,
  * '\\' joins the next line, newlines inside { } are joined,
  * lowercase a-z OUTSIDE double quotes are deleted, runs of blanks collapse to one.
Then every top-level command word and every command word inside ONTAG bodies is counted.
Words that are not in the engine's console/INI command table are flagged with '!'.
"""
import collections
import io
import os
import re
import sys
import zipfile

# Console / INI command table of lips.exe (PTR table @0x0050fa48, name -> handler).
ENGINE_COMMANDS = set("""
FLYON FLYOFF GOTO PLAY STOP PAUSE SOUNDON SOUNDOFF PHYSICSON PHYSICSOFF SET TRACE TAG WATCH
DOORBELL SHOW INVERTMOUSE SYSINFO CHECKWDIST NEW SPEAK JOIN NAME QUIT LOG MAIL MAILBUG
MAILCOMMENT MAILTO MAILFROM MAILSUBJECT KICKBAN GAMEPASSWORD NETTAG KEYMAP KEYBIND EXECUTE
OPEN OSETJUMP OSETCROU OSETZSPD OSETFLY OPRSTON ODRSTON OPHYSON OSETPOS OSETSPD OMAXOS OSETOS
OINCOS OINCTOOS OENGINE OSETGEN OSETVIEW OSETVIS OSTEPON OSETKEYS RPORTAL LSETTINT LSETFADE
WEBCHATNEW WEBCHATOFF SETMINOS SETMINOSTAGS SETMAXOS SETMAXOSTAGS SETLOWOS SETLOWOSTAGS
SETHIGHOS SETHIGHOSTAGS ONTAG OTEXT OSOUND OSETMOVE INITMOVES INITPARTSETS INCOS SUMOS
INCTOOS SETOS XCHANGE INITTOOL TOOLSOUT OPTOOL OTOOLCON OPPROJ OPXPLD OOTOOL OOPROJ OOXPLD
AVAILTOOLS OPPONENT XM XT NOSEEUM STATEVAL WTX WALLTX WTY WALLTY WFADE WALLFADE WTEX WALLTEX
FTEX FFADE CTEX CFADE
""".split())

COMMAND_SECTIONS = {"COMMANDS9999", "NOSTRAP9999"}
WORD_DELIMS = set(' ,;?({)}="\n\r')   # tokenizer delimiters (string @0x00409381)


def read_sources(game_dir):
    for name in sorted(os.listdir(game_dir)):
        path = os.path.join(game_dir, name)
        if name.lower().endswith(".ini"):
            with open(path, "rb") as f:
                yield name, f.read().decode("latin1")
        elif name.upper() == "LIPSINI.ZIP":
            with zipfile.ZipFile(path) as z:
                for info in sorted(z.infolist(), key=lambda i: i.filename):
                    if info.filename.lower().endswith(".ini"):
                        yield f"LIPSINI.ZIP:{info.filename}", z.read(info).decode("latin1")


def split_sections(text):
    sections = collections.OrderedDict()
    cur = None
    for raw in io.StringIO(text):
        line = raw.rstrip("\r\n")
        m = re.match(r"\s*\[([^\]]+)\]", line)
        if m:
            cur = m.group(1).strip().upper()
            sections.setdefault(cur, [])
            continue
        if cur is not None:
            sections[cur].append(line)
    return sections


def preprocess(lines):
    """Mimic FUN_00449f3b: returns the list of logical command lines."""
    out, buf, depth, cont = [], [], 0, False
    for line in lines:
        semi = line.find(";")
        if semi >= 0:
            line = line[:semi]
        bs = line.find("\\")
        joined = bs >= 0
        if joined:
            line = line[:bs]
        for ch in line:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth = max(0, depth - 1)
        buf.append(line)
        if joined or depth > 0:
            continue
        out.append(" ".join(buf))
        buf = []
    if buf:
        out.append(" ".join(buf))
    result = []
    for line in out:
        s, inq, prev_space = [], False, False
        for ch in line.strip():
            if inq:
                s.append(ch)
                if ch == '"':
                    inq = False
                continue
            if ch == " ":
                if prev_space:
                    continue
                prev_space = True
            else:
                prev_space = False
            if "a" <= ch <= "z":
                continue                      # lowercase outside quotes is deleted
            if ch == '"':
                inq = True
            s.append(ch)
        if s:
            result.append("".join(s).strip())
    return result


def first_word(text):
    text = text.lstrip(" =({,")
    w = []
    for ch in text:
        if ch in WORD_DELIMS:
            break
        w.append(ch)
    return "".join(w).upper()


def body_commands(body):
    """Command words of an ONTAG body: walk balanced (...) groups like FUN_0040b545."""
    words, i, n = [], 0, len(body)
    body = body.strip().lstrip("{").rstrip("}")
    n = len(body)
    while i < n:
        while i < n and body[i] in ' ,;?=':
            i += 1
        j = i
        while j < n and body[j] not in WORD_DELIMS:
            j += 1
        word = body[i:j].upper()
        if not word:
            if j < n and body[j] in '({':
                j += 1
            i = j + 1 if j == i else j
            continue
        words.append(word)
        k = j
        while k < n and body[k] == " ":
            k += 1
        if k < n and body[k] in "({":
            open_c = body[k]
            close_c = ")" if open_c == "(" else "}"
            depth, inq = 0, False
            while k < n:
                c = body[k]
                if c == '"':
                    inq = not inq
                elif not inq and c == open_c:
                    depth += 1
                elif not inq and c == close_c:
                    depth -= 1
                    if depth == 0:
                        k += 1
                        break
                k += 1
        i = k
    return words


def main(game_dir="original"):
    sections_seen = collections.Counter()
    startup_keys = collections.Counter()
    top_cmds = collections.Counter()
    ontag_cmds = collections.Counter()
    named_keys = collections.Counter()
    mover_names = collections.Counter()
    mover_tokens = collections.Counter()
    per_file = {}
    for fname, text in read_sources(game_dir):
        secs = split_sections(text)
        per_file[fname] = list(secs)
        for sec, lines in secs.items():
            sections_seen[sec] += 1
            if sec == "STARTUP":
                for l in lines:
                    m = re.match(r"\s*([A-Za-z0-9_]+)\s*=", l)
                    if m and not l.lstrip().startswith(";"):
                        startup_keys[m.group(1).lower()] += 1
            elif sec in COMMAND_SECTIONS:
                for cl in preprocess(lines):
                    w = first_word(cl)
                    base = w.split("[")[0]
                    top_cmds[base] += 1
                    if base == "ONTAG":
                        body = cl.split("=", 1)[1] if "=" in cl else ""
                        for bw in body_commands(body):
                            ontag_cmds[bw] += 1
            elif sec == "NAMEDMESSAGES":
                for l in lines:
                    if l.lstrip().startswith((";", "/")) or "=" not in l:
                        continue
                    key = l.split("=", 1)[0].strip().upper()
                    named_keys[re.sub(r"^M[0-9A-Z]", "M?", key)] += 1
            elif sec == "MOVERACTIONCODES":
                for cl in preprocess(lines):
                    if "=" not in cl:
                        continue
                    name, code = cl.split("=", 1)
                    mover_names[name.strip()] += 1
                    code = re.sub(r'"[^"]*"', '""', code)
                    for tok in re.findall(r"\b(ON [A-Z]{1,2}|[A-Z][A-Z0-9]*)\s*(?=[({]|\s|\*|$)", code):
                        mover_tokens[tok] += 1

    def dump(title, counter, flag=False):
        print(f"\n== {title} ({len(counter)} distinct)")
        for k, v in sorted(counter.items(), key=lambda kv: (-kv[1], kv[0])):
            mark = "!" if flag and k not in ENGINE_COMMANDS else " "
            print(f"  {mark}{k:<16} {v}")

    print("== files")
    for f, secs in per_file.items():
        print(f"  {f:<26} {' '.join(secs)}")
    dump("sections", sections_seen)
    dump("[startup] keys", startup_keys)
    dump("[commands9999]/[nostrap9999] top-level commands", top_cmds, flag=True)
    dump("commands inside ONTAG[n] bodies", ontag_cmds, flag=True)
    dump("[namedmessages] keys (menu id folded to M?)", named_keys)
    dump("[moveractioncodes] script names", mover_names)
    dump("[moveractioncodes] tokens (after pre-processing; heuristic)", mover_tokens)
    print("\n'!' = not a command in the lips.exe command table: the rest of that line/ONTAG body is skipped.")


if __name__ == "__main__":
    main(*sys.argv[1:])
