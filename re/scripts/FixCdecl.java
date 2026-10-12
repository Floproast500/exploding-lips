// Watcom's variadic/CRT functions are caller-cleanup stack functions. Any function whose call
// sites are followed by "ADD ESP, imm" gets __cdecl instead of __watcall.
//@category Lips
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.function.DecompilerParameterIdCmd;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;

public class FixCdecl extends GhidraScript {
    @Override
    public void run() throws Exception {
        Listing listing = currentProgram.getListing();
        ReferenceManager refs = currentProgram.getReferenceManager();
        AddressSet entries = new AddressSet();
        for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
            String cc = f.getCallingConventionName();
            if (!"__watcall".equals(cc) && !"__cdecl".equals(cc)) continue;
            int calls = 0, cleaned = 0;
            for (Reference r : refs.getReferencesTo(f.getEntryPoint())) {
                if (!r.getReferenceType().isCall()) continue;
                calls++;
                Instruction call = listing.getInstructionAt(r.getFromAddress());
                Instruction next = call == null ? null : call.getNext();
                if (next != null && next.getMnemonicString().equals("ADD")
                        && next.getDefaultOperandRepresentation(0).equals("ESP")) cleaned++;
            }
            if (calls > 0 && cleaned * 2 > calls) {
                f.setCallingConvention("__cdecl");
                f.setVarArgs(true);
                entries.add(f.getEntryPoint());
                println("cdecl: " + f.getName() + " (" + cleaned + "/" + calls + ")");
            }
        }
        new DecompilerParameterIdCmd("cdecl param id", entries, SourceType.ANALYSIS, false, false, 60)
            .applyTo(currentProgram, monitor);
    }
}
