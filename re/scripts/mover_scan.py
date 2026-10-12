"""Inventory of the mover ("bot") script language used by the shipped Exploding Lips INIs.

usage: python3 -I re/scripts/mover_scan.py [GAME_DIR] [--scripts]

Reads every *.INI / *.ini in GAME_DIR (default ./original) plus the copies inside LIPSINI.ZIP
(read in memory, nothing is extracted), takes the [moveractioncodes] section, normalises it the
way lips.exe does (re/engine/script.md section 2: ';' comments, '\\' continuation, newlines
inside { } joined, lowercase letters outside quotes deleted, blank runs collapsed) and then
splits each NAME=code line with the same rules as the engine's compiler/interpreter
(re/engine/movers.md):

  code   := [IN '{' body '}'] (ON cond '{' body '}'){0..4} body
  body   := { item }        item := '{' | '}' ['(' n ')'] | '*' | word [ '(' args ')' ]
  word   >= 4 characters  -> ordinary INI command (OSOUND, OTEXT, XCHANGE, ...)
  word   <  4 characters  -> mover opcode, dispatched on its first letter

Prints counts of: ON conditions, mover opcodes (and where they occur: IN / ON / main), INI
commands embedded in scripts, structural tokens, plus the number of files and distinct scripts.
With --scripts every distinct script is printed in decoded form.
"""
import collections
import io
import os
import re
import sys
import zipfile

WORD_DELIMS = set(' ,;?({)}="\n\r')          # command-word delimiters (data @0x00409381)
MOVE_LETTERS = set("UDFBRLH")                 # direct movement opcodes (FUN_00447ef5)
TWO_LETTER = set("AGMSTWX")                   # first letter + one sub-letter
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

# ON-condition letters (FUN_00444dc8): event letter + optional actor letter (FUN_00444cef).
COND_EVENTS = {"B": "bumped", "H": "hit", "C": "close", "V": "sees", "E": "E-flag", "L": "L-flag",
               "F": "F(n)", "M": "M(n)", "S": "S(n)", "T": "tag", "U": "chat text", "X": "error"}
COND_ACTORS = {"P": "player", "O": "object", "N": "netplayer", "A": "any", "K": "kill",
               "W": "wall", "C": "C-actor", "F": "F-actor"}


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
                        yield "LIPSINI.ZIP:" + info.filename, z.read(info).decode("latin1")


def section_lines(text, wanted):
    cur, out = None, []
    for raw in io.StringIO(text):
        line = raw.rstrip("\r\n")
        m = re.match(r"\s*\[([^\]]+)\]", line)
        if m:
            cur = m.group(1).strip().upper()
            continue
        if cur == wanted:
            out.append(line)
    return out


def preprocess(lines):
    """Logical lines after the engine's FUN_00449f3b normalisation."""
    joined, buf, depth = [], [], 0
    for line in lines:
        semi = line.find(";")
        if semi >= 0:
            line = line[:semi]
        bs = line.find("\\")
        cont = bs >= 0
        if cont:
            line = line[:bs]
        for ch in line:
            if ch == "{":
                depth = min(depth + 1, 16)
            elif ch == "}":
                depth = max(0, depth - 1)
        buf.append(line)
        if cont or depth > 0:
            continue
        joined.append(" ".join(buf))
        buf = []
    if buf:
        joined.append(" ".join(buf))
    result = []
    for line in joined:
        s, inq, prev_space = [], False, False
        for ch in line.strip():
            if inq:
                s.append(ch)
                if ch == '"':
                    inq = False
                continue
            if ch in " \t":
                if prev_space:
                    continue
                prev_space = True
                s.append(" ")
                continue
            prev_space = False
            if "a" <= ch <= "z":
                continue
            if ch == '"':
                inq = True
            s.append(ch)
        t = "".join(s).strip()
        if t:
            result.append(t)
    return result


def match_brace(s, i):
    """s[i] == '{' -> index of the matching '}' (or len(s))."""
    depth = 0
    while i < len(s):
        if s[i] == "{":
            depth += 1
        elif s[i] == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(s)


def skip_group(s, i):
    """Skip blanks and one balanced '(...)' group starting at/after i (quotes respected)."""
    k = i
    while k < len(s) and s[k] == " ":
        k += 1
    if k >= len(s) or s[k] != "(":
        return i, None
    depth, inq, start = 0, False, k
    while k < len(s):
        c = s[k]
        if c == '"':
            inq = not inq
        elif not inq and c == "(":
            depth += 1
        elif not inq and c == ")":
            depth -= 1
            if depth == 0:
                return k + 1, s[start + 1:k]
        k += 1
    return k, s[start + 1:]


def parse_condition(text):
    """Condition letters between ON and '{' -> list of normalised tokens."""
    toks, i = [], 0
    t = text.strip()
    while i < len(t):
        c = t[i]
        if c in "BCHV":
            actor = t[i + 1] if i + 1 < len(t) and t[i + 1] in COND_ACTORS else ""
            toks.append(c + actor)
            i += 2 if actor else 1
        elif c in "FMST":
            j, arg = skip_group(t, i + 1)
            toks.append(f"{c}({arg})" if arg is not None else c)
            i = j
        elif c == "U":
            m = re.match(r'U\s*\(?\s*"[^"]*"\)?', t[i:])
            toks.append("U\"..\"")
            i += m.end() if m else 1
        elif c in "EL":
            toks.append(c)
            i += 1
        elif c == "X":
            i += 2 if i + 1 < len(t) and t[i + 1] in "~F" else 1
            toks.append("X")
        else:
            i += 1
    return toks


def tokenize_body(s):
    """Items of a body in interpreter order: ('op', code, args) / ('cmd', name, args) / ('sym', c)."""
    items, i = [], 0
    while i < len(s):
        c = s[i]
        if c == " ":
            i += 1
            continue
        if c in "{*":
            items.append(("sym", c, None))
            i += 1
            continue
        if c == "}":
            j, arg = skip_group(s, i + 1)
            items.append(("sym", "}" if arg is None else "}(" + arg + ")", None))
            i = j
            continue
        if c in "),":
            items.append(("sym", "stray" + c, None))
            i += 1
            continue
        j = i
        while j < len(s) and s[j] not in WORD_DELIMS:
            j += 1
        word = s[i:j]
        if not word:
            i += 1
            continue
        if len(word) >= 4:
            j2, arg = skip_group(s, j)
            known = word.split("[")[0] in ENGINE_COMMANDS
            items.append(("cmd" if known else "badcmd", word, arg))
            i = j2
            continue
        code = word
        if code[0] == "~":
            code = code[1:]
        j2, arg = skip_group(s, j)
        items.append(("op", normalise_op(code), arg))
        i = j2
    return items


def normalise_op(code):
    c = code[0]
    if c in MOVE_LETTERS:
        return code if all(ch in MOVE_LETTERS for ch in code) else code[0] + "?"
    if c == "P":
        return code[:3]
    if c in TWO_LETTER:
        return code[:2]
    return code + "(?)"


def compile_script(code):
    """Split a script into (in_body, [(cond, body)], main_body) like FUN_0044a3f8."""
    s = code.strip()
    in_body, ons = None, []
    if s.startswith("IN"):
        k = s.find("{", 2)
        if k >= 0:
            e = match_brace(s, k)
            in_body = s[k + 1:e]
            s = s[e + 1:].strip()
    while s.startswith("ON") and len(ons) < 4:
        k = s.find("{", 2)
        if k < 0:
            break
        e = match_brace(s, k)
        ons.append((parse_condition(s[2:k]), s[k + 1:e]))
        s = s[e + 1:].strip()
        m = re.match(r"\(\s*\d+\s*\)", s)
        if m:
            s = s[m.end():].strip()
    return in_body, ons, s


def main(argv):
    game_dir, show = "original", False
    for a in argv:
        if a == "--scripts":
            show = True
        else:
            game_dir = a
    files = 0
    ops, ops_where, cmds, conds, syms = (collections.Counter() for _ in range(5))
    uops, ucmds, uconds = collections.Counter(), collections.Counter(), collections.Counter()
    op_files = collections.defaultdict(set)
    distinct = collections.OrderedDict()
    key_files = collections.defaultdict(list)
    names = collections.Counter()
    for fname, text in read_sources(game_dir):
        lines = section_lines(text, "MOVERACTIONCODES")
        if not lines:
            continue
        files += 1
        for cl in preprocess(lines):
            if "=" not in cl:
                names["(no '=' : '%s')" % cl[:20]] += 1
                continue
            name, code = cl.split("=", 1)
            name = name.strip()[:11]
            names[name] += 1
            key = (name, code.strip())
            first = key not in distinct
            key_files[key].append(fname)
            in_body, ons, main_body = compile_script(code)
            distinct.setdefault(key, (in_body, ons, main_body))
            parts = []
            if in_body is not None:
                syms["IN"] += 1
                parts.append(("IN", in_body))
            for cond, body in ons:
                syms["ON"] += 1
                c = "ON " + " ".join(cond)
                conds[c] += 1
                if first:
                    uconds[c] += 1
                parts.append(("ON", body))
            parts.append(("main", main_body))
            for where, body in parts:
                for kind, tok, arg in tokenize_body(body):
                    if kind == "op":
                        ops[tok] += 1
                        ops_where[(tok, where)] += 1
                        op_files[tok].add(fname)
                        if first:
                            uops[tok] += 1
                    elif kind in ("cmd", "badcmd"):
                        label = tok if kind == "cmd" else "!" + tok
                        cmds[label] += 1
                        if first:
                            ucmds[label] += 1
                    else:
                        syms[tok] += 1

    print(f"files with [moveractioncodes]: {files}")
    print(f"script definitions: {sum(names.values())}  distinct (name, text): {len(distinct)}  "
          f"distinct names: {len(names)}")

    def dump(title, counter, ucounter=None, extra=None):
        print(f"\n== {title}")
        print(f"   {'token':<14} {'all copies':>10} {'distinct':>9}")
        for k, v in sorted(counter.items(), key=lambda kv: (-kv[1], kv[0])):
            u = ucounter[k] if ucounter is not None else ""
            x = extra(k) if extra else ""
            print(f"   {k:<14} {v:>10} {u!s:>9}  {x}")

    dump("ON conditions", conds, uconds)

    def where(tok):
        w = [f"{p}:{ops_where[(tok, p)]}" for p in ("IN", "ON", "main") if ops_where[(tok, p)]]
        return f"[{' '.join(w)}] files={len(op_files[tok])}"
    dump("mover opcodes", ops, uops, where)
    dump("INI commands inside mover scripts ('!' = not a command: falls back to opcode dispatch)",
         cmds, ucmds)
    dump("structure", syms)
    print("\n== script names (definitions over all copies)")
    print("   " + ", ".join(f"{k}x{v}" for k, v in sorted(names.items())))

    variants = collections.defaultdict(list)
    for key in distinct:
        variants[key[0]].append(key)
    multi = {n: ks for n, ks in variants.items() if len(ks) > 1}
    if multi:
        print("\n== scripts with more than one variant (files carrying each variant)")
        for n, ks in multi.items():
            for k in ks:
                fl = key_files[k]
                print(f"   {n:<10} {len(fl):>2} files: {' '.join(fl) if len(fl) < 6 else '(all others)'}")

    if show:
        print("\n== distinct scripts (decoded)")
        for (name, _), (in_body, ons, main_body) in distinct.items():
            print(f"\n{name}:")
            if in_body is not None:
                print("   IN   " + fmt(tokenize_body(in_body)))
            for cond, body in ons:
                print(f"   ON {' '.join(cond):<4} " + fmt(tokenize_body(body)))
            print("   MAIN " + fmt(tokenize_body(main_body)))


def fmt(items):
    out = []
    for kind, tok, arg in items:
        if kind == "sym":
            out.append(tok)
        else:
            out.append(("!" if kind == "badcmd" else "") + tok + ("" if arg is None else f"({arg})"))
    return " ".join(out)


if __name__ == "__main__":
    main(sys.argv[1:])
