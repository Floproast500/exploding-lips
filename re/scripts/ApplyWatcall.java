// Add a Watcom register calling convention (__watcall) as a spec extension, apply it to
// internal functions that are only ever called directly, then re-run parameter ID.
//@category Lips
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.function.DecompilerParameterIdCmd;
import ghidra.program.database.SpecExtension;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;

public class ApplyWatcall extends GhidraScript {
    static final String PROTO =
        "<prototype name=\"__watcall\" extrapop=\"unknown\" stackshift=\"4\">" +
        " <input>" +
        "  <pentry minsize=\"1\" maxsize=\"4\"><register name=\"EAX\"/></pentry>" +
        "  <pentry minsize=\"1\" maxsize=\"4\"><register name=\"EDX\"/></pentry>" +
        "  <pentry minsize=\"1\" maxsize=\"4\"><register name=\"EBX\"/></pentry>" +
        "  <pentry minsize=\"1\" maxsize=\"4\"><register name=\"ECX\"/></pentry>" +
        "  <pentry minsize=\"1\" maxsize=\"500\" align=\"4\"><addr offset=\"4\" space=\"stack\"/></pentry>" +
        " </input>" +
        " <output>" +
        "  <pentry minsize=\"4\" maxsize=\"10\" metatype=\"float\"><register name=\"ST0\"/></pentry>" +
        "  <pentry minsize=\"1\" maxsize=\"4\"><register name=\"EAX\"/></pentry>" +
        " </output>" +
        " <unaffected>" +
        "  <register name=\"EBX\"/><register name=\"ECX\"/><register name=\"EDX\"/>" +
        "  <register name=\"ESI\"/><register name=\"EDI\"/><register name=\"EBP\"/><register name=\"ESP\"/>" +
        " </unaffected>" +
        "</prototype>";

    @Override
    public void run() throws Exception {
        new SpecExtension(currentProgram).addReplaceCompilerSpecExtension(PROTO, monitor);

        ReferenceManager refs = currentProgram.getReferenceManager();
        AddressSet entries = new AddressSet();
        int applied = 0, skipped = 0;
        for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
            if (f.isThunk() || f.isExternal()) continue;
            // Functions whose address is taken (window procs, thread procs, callbacks) keep stdcall.
            boolean addressTaken = false;
            for (Reference r : refs.getReferencesTo(f.getEntryPoint())) {
                if (!r.getReferenceType().isCall() && !r.getReferenceType().isJump()) { addressTaken = true; break; }
            }
            if (addressTaken) { skipped++; continue; }
            f.setCallingConvention("__watcall");
            f.setCustomVariableStorage(false);
            entries.add(f.getEntryPoint());
            applied++;
        }
        println("watcall applied=" + applied + " skipped(address-taken)=" + skipped);
        DecompilerParameterIdCmd cmd = new DecompilerParameterIdCmd("watcall param id", entries,
            SourceType.ANALYSIS, false, false, 60);
        cmd.applyTo(currentProgram, monitor);
    }
}
