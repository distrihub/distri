// Imports a sibling module: editing it must re-extract this tool.
import { defineTool } from "jsr:@distri/tool";
import { DESCRIPTION } from "./greeting_lib.ts";

export const tool = defineTool({ name: "uses_lib", description: DESCRIPTION });

export default () => ({ description: DESCRIPTION });
