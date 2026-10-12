// Create functions in the code gaps Ghidra's auto-analysis left (event handlers reached only via
// tables, renderer helpers, ...), then decompile just the new functions.
// Args: <outdir>.  Writes decomp_extra.c and functions_extra.tsv. Run with -readOnly.
//@category Lips
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import java.io.*;
import java.util.*;

public class FillGaps extends GhidraScript {
    @Override
    public void run() throws Exception {
        String outDir = getScriptArgs()[0];
        Memory mem = currentProgram.getMemory();
        Address start = toAddr(0x401000), end = toAddr(0x50c3f0);
        List<Function> created = new ArrayList<>();
        boolean progress = true;
        int pass = 0;
        while (progress && pass < 20) {
            progress = false; pass++;
            Address a = start;
            while (a.compareTo(end) < 0) {
                Function f = getFunctionContaining(a);
                if (f != null) { a = f.getBody().getMaxAddress().add(1); continue; }
                int b = mem.getByte(a) & 0xff;
                if (b == 0x00 || b == 0x90 || b == 0xcc) { a = a.add(1); continue; }
                // skip data embedded: require plausible prologue byte
                if (!(b == 0x53 || b == 0x51 || b == 0x52 || b == 0x56 || b == 0x57 || b == 0x55 || b == 0x50 || b == 0x60 || b == 0x8b || b == 0x83 || b == 0x31 || b == 0xb8 || b == 0x68)) { a = a.add(1); continue; }
                new DisassembleCommand(a, null, true).applyTo(currentProgram, monitor);
                Function nf = null;
                try { nf = createFunction(a, null); } catch (Exception e) {}
                if (nf == null) { a = a.add(1); continue; }
                try { nf.setCallingConvention("__watcall"); } catch (Exception e) {}
                created.add(nf); progress = true;
                a = nf.getBody().getMaxAddress().add(1);
            }
        }
        DecompInterface ifc = new DecompInterface();
        ifc.openProgram(currentProgram);
        created.sort((x, y) -> x.getEntryPoint().compareTo(y.getEntryPoint()));
        try (PrintWriter c = new PrintWriter(new FileWriter(outDir + "/decomp_extra.c"));
             PrintWriter idx = new PrintWriter(new FileWriter(outDir + "/functions_extra.tsv"))) {
            for (Function f : created) {
                idx.printf("%s\t%s\t%d%n", f.getEntryPoint(), f.getName(), f.getBody().getNumAddresses());
                DecompileResults r = ifc.decompileFunction(f, 60, monitor);
                c.printf("// ==== %s @ %s ====%n", f.getName(), f.getEntryPoint());
                c.println(r.decompileCompleted() ? r.getDecompiledFunction().getC() : "// decompile failed: " + r.getErrorMessage());
            }
        }
        println("created " + created.size());
        ifc.dispose();
    }
}
