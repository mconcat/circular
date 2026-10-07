/** Optional task text; all recovery, execution and approval belong to circular edit. */
export const task = `Edit the Agent Session Monitor using the Circular SDK.
Preserve the normalized {signal, recognized, evidence} contract and both agent-observed-* mounts.
Keep source normalization upstream of classify_logs/classify_metrics.
For HTTP pull, use timer and request with SDK map/parse combinators. A response body is Bytes;
convert it with {'body': string(event.body)} before parsing the String body.
Use SDK Int literals such as 100n for timer intervals. Consult published admission for ports/config.`;
