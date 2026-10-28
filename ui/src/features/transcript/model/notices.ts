/** Plain access-change notices from Vibe history, including older entries without structured names.
 * Depends on history accessors and the shared access menu labels; called by NoticeRow and DividerRow.
 */
import { TEXT } from "../../../vocabulary";
import { modeOf } from "../../composer/model/access";
import { type Obj, detailOf, obj, str } from "./entries";

/** Display a structured access change with the same words as its picker. */
export function noticeText(entry: Obj): string {
  const message = str(entry["message"]) ?? "";
  const detail = detailOf(entry);
  if (detail["kind"] !== "agent_changed" && entry["kind"] !== "agent_change") return message;
  const agent =
    str(detail["agentName"]) ??
    str(obj(entry["details"])?.["agent"]) ??
    /^Agent changed to (.+)$/.exec(message)?.[1] ??
    null;
  return `${TEXT.accessMode} changed to ${modeOf(agent)?.label ?? TEXT.accessUnknown}.`;
}
