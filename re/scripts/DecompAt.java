// Decompile functions at the given addresses (creating them if Ghidra missed them).
// Args: <outfile> <hexaddr>...   Run with -readOnly so nothing is saved.
//@category Lips
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.io.*;

public class DecompAt extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] a = getScriptArgs();
        DecompInterface ifc = new DecompInterface();
        ifc.openProgram(currentProgram);
        try (PrintWriter c = new PrintWriter(new FileWriter(a[0]))) {
            for (int i = 1; i < a.length; i++) {
                Address ad = toAddr(a[i]);
                Function f = getFunctionAt(ad);
                if (f == null) {
                    new DisassembleCommand(ad, null, true).applyTo(currentProgram, monitor);
                    f = createFunction(ad, null);
                    if (f != null) {
                        try { f.setCallingConvention("__watcall"); } catch (Exception e) {}
                    }
                }
                if (f == null) { c.printf("// ==== %s: could not create function%n", a[i]); continue; }
                DecompileResults r = ifc.decompileFunction(f, 120, monitor);
                c.printf("// ==== %s @ %s ====%n", f.getName(), f.getEntryPoint());
                c.println(r.decompileCompleted() ? r.getDecompiledFunction().getC() : "// decompile failed: " + r.getErrorMessage());
            }
        }
        ifc.dispose();
    }
}
