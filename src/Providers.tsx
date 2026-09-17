import { useEffect, useRef, useState } from "react";
import { commands, type ProviderId, type ProviderInstallationView } from "./bindings";

const supported = ["claude", "codex"] as const;
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

export default function Providers() {
  const [rows, setRows] = useState<ProviderInstallationView[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<ProviderId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const mounted = useRef(false);
  const request = useRef(0);
  const detecting = useRef(false);

  async function load() {
    const version = ++request.current;
    setLoading(true);
    setError(null);
    try {
      const result = await commands.listProviderInstallations();
      if (!mounted.current || version !== request.current) return;
      if (result.status === "error") throw new Error(result.error);
      setRows(result.data);
    } catch (err) {
      if (mounted.current && version === request.current) setError(errorText(err));
    } finally {
      if (mounted.current && version === request.current) setLoading(false);
    }
  }
  useEffect(() => {
    mounted.current = true;
    void load();
    return () => { mounted.current = false; };
  }, []);

  async function detect(provider: typeof supported[number]) {
    if (detecting.current || loading) return;
    detecting.current = true;
    setBusy(provider);
    setError(null);
    setNotice("");
    try {
      const result = await commands.detectProvider({ provider_id: provider });
      if (!mounted.current) return;
      if (result.status === "error") throw new Error(result.error);
      setRows(current => [...current.filter(row => !(row.provider === provider && row.runtime === "native_windows")), result.data]);
      setNotice(`${provider} detection saved. Authentication and model availability have not been checked.`);
    } catch (err) {
      if (mounted.current) setError(errorText(err));
    } finally {
      detecting.current = false;
      if (mounted.current) setBusy(null);
    }
  }

  return <section className="providers role-matrix" aria-labelledby="providers-title">
    <h2 id="providers-title">Providers</h2>
    <p>Native Windows installation observations only—not authentication, model availability, or readiness.</p>
    <p>Detect runs the registered CLI’s version command (15-second limit). Command names below are not resolved absolute paths.
      An unsuccessful probe may mean a missing or broken launcher.</p>
    {error && <p role="alert">{error}</p>}
    <p role="status">{notice}</p>
    <div className="role-actions">
      <button disabled={loading || busy !== null} onClick={() => void load()}>Reload saved detections</button>
      {supported.map(provider => <button key={provider} disabled={loading || busy !== null} onClick={() => void detect(provider)}>
        {busy === provider ? `Detecting ${provider}…` : `Detect ${provider}`}
      </button>)}
    </div>
    {loading ? <p role="status">Loading saved detections…</p> : rows.length === 0 ? <p>No saved provider detections.</p> :
      <div className="role-table-wrap"><table>
        <caption>Last saved observations—use Detect to refresh</caption>
        <thead><tr><th scope="col">Provider / runtime</th><th scope="col">Observation</th><th scope="col">Command / version</th><th scope="col">Detected at (Unix milliseconds)</th></tr></thead>
        <tbody>{rows.map(row => <tr key={`${row.provider}-${row.runtime}`}>
          <th scope="row">{row.provider}<small>{row.runtime}</small></th>
          <td>{row.installed ? "Version probe succeeded" : "Not detected or probe unsuccessful"}</td>
          <td>{row.executable_path ?? "Unavailable"}<small>{row.version ?? "Version unavailable"}</small></td>
          <td>{row.detected_at_millis}</td>
        </tr>)}</tbody>
      </table></div>}
    <p>Antigravity, Copilot, OpenCode, WSL2, and Docker detection are not wired into this panel yet.
      No credentials are read and no agent session is started by these controls.</p>
  </section>;
}
