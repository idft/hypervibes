import { Link } from "@tanstack/react-router";
import type { MemoryReplacements as Replacements } from "../memory/memory-contracts.js";

/** Replacement standing is separate from the evidence-age badge. */
export function MemoryReplacements({ agentId, memory }: { agentId: string; memory: Replacements }) {
  if (!memory.correctedBy && !memory.supersededBy) return null;
  return <div aria-label="Memory replacements" className="mt-2 flex flex-col gap-1 text-xs text-amber-300">
    {memory.correctedBy && <Link to="/agents/$agentId/memories/$memoryRef" params={{ agentId, memoryRef: memory.correctedBy }} className="cursor-pointer hover:underline">Corrected by: {memory.correctedBy}</Link>}
    {memory.supersededBy && <Link to="/agents/$agentId/memories/$memoryRef" params={{ agentId, memoryRef: memory.supersededBy }} className="cursor-pointer hover:underline">Superseded by: {memory.supersededBy}</Link>}
  </div>;
}
