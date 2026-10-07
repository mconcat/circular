import { exactPayloadPathIssue } from "../../protocol/src/payload-path.js";
export function joinConfigIssue(config) {
  if (!config || typeof config !== "object" || Array.isArray(config) || !Object.hasOwn(config, "at")) {
    return "join config.at: required exact payload path is missing";
  }
  const issue = exactPayloadPathIssue(config.at);
  return issue ? `join config.at: ${issue}` : null;
}
