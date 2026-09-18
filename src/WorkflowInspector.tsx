import { useEffect, useRef, useState } from "react";
import {
  commands,
  type WorkflowNode,
  type WorkflowTemplateDefinitionView,
  type WorkflowTemplateView,
} from "./bindings";
import "./OperationalSurfaces.css";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export default function WorkflowInspector() {
  const [templates, setTemplates] = useState<WorkflowTemplateView[]>([]);
  const [selectedName, setSelectedName] = useState<string>("");
  const [definition, setDefinition] = useState<WorkflowTemplateDefinitionView | null>(null);
  const [templatesLoading, setTemplatesLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const loadVersion = useRef(0);
  const detailVersion = useRef(0);

  async function loadTemplates() {
    const version = ++loadVersion.current;
    setTemplatesLoading(true);
    setError(null);
    try {
      const result = await commands.listWorkflowTemplates();
      if (!mounted.current || version !== loadVersion.current) return;
      if (result.status === "error") throw new Error(result.error);
      setTemplates(result.data);
      if (result.data.length > 0) {
        setSelectedName((current) =>
          result.data.some((t) => t.name === current)
            ? current
            : result.data[0]?.name ?? ""
        );
      } else {
        setSelectedName("");
        setDefinition(null);
      }
    } catch (err) {
      if (mounted.current && version === loadVersion.current) {
        setError(`Failed to list workflow templates: ${message(err)}`);
      }
    } finally {
      if (mounted.current && version === loadVersion.current) {
        setTemplatesLoading(false);
      }
    }
  }

  async function inspectTemplate(name: string) {
    if (!name) {
      setDefinition(null);
      return;
    }
    const version = ++detailVersion.current;
    setDetailLoading(true);
    setError(null);
    try {
      const result = await commands.getWorkflowTemplate({ name });
      if (!mounted.current || version !== detailVersion.current) return;
      if (result.status === "error") throw new Error(result.error);
      setDefinition(result.data);
    } catch (err) {
      if (mounted.current && version === detailVersion.current) {
        setError(`Failed to inspect template '${name}': ${message(err)}`);
      }
    } finally {
      if (mounted.current && version === detailVersion.current) {
        setDetailLoading(false);
      }
    }
  }

  useEffect(() => {
    mounted.current = true;
    void loadTemplates();
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (selectedName) {
      void inspectTemplate(selectedName);
    }
  }, [selectedName]);

  return (
    <section className="operational-surface" aria-labelledby="workflow-inspector-title">
      <header className="surface-header">
        <h2 id="workflow-inspector-title">Workflow Template Inspector</h2>
        <p className="surface-note">
          Read-only inspection of workflow graphs and DAG nodes. Built-in presets are protected by
          runtime policy; template mutations require human review and engine-level validation.
        </p>
      </header>

      {error && (
        <div role="alert" className="surface-alert" data-testid="workflow-inspector-error">
          {error}
        </div>
      )}

      <div className="surface-controls">
        <label htmlFor="workflow-template-select">
          Select Workflow Template
          <select
            id="workflow-template-select"
            data-testid="workflow-template-select"
            value={selectedName}
            disabled={templatesLoading || templates.length === 0}
            onChange={(e) => setSelectedName(e.target.value)}
          >
            {templates.map((tpl) => (
              <option key={tpl.name} value={tpl.name}>
                {tpl.name} {tpl.is_built_in ? "(Built-in)" : "(Custom)"}
              </option>
            ))}
          </select>
        </label>

        <button
          type="button"
          data-testid="refresh-templates-button"
          disabled={templatesLoading || detailLoading}
          onClick={() => void loadTemplates()}
        >
          {templatesLoading ? "Refreshing..." : "Refresh Templates"}
        </button>
      </div>

      {templatesLoading && !definition && (
        <p data-testid="workflow-inspector-loading">Loading workflow templates...</p>
      )}

      {!templatesLoading && templates.length === 0 && (
        <div className="surface-empty" data-testid="workflow-inspector-empty">
          No workflow templates found in database or built-in registry.
        </div>
      )}

      {detailLoading && <p data-testid="workflow-detail-loading">Loading template definition...</p>}

      {definition && (
        <article className="surface-card" data-testid="workflow-template-card">
          <header style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <div>
              <h3 data-testid="template-name">{definition.name}</h3>
              <p style={{ margin: "0.25rem 0", opacity: 0.9 }}>{definition.description}</p>
            </div>
            <div>
              <span
                className={`surface-badge ${definition.is_built_in ? "badge-builtin" : "badge-custom"}`}
                data-testid="template-builtin-badge"
              >
                {definition.is_built_in ? "Built-in (Protected)" : "Custom Template"}
              </span>
              <span style={{ marginLeft: "0.5rem", fontSize: "0.85rem", opacity: 0.8 }}>
                v{definition.version}
              </span>
            </div>
          </header>

          <section style={{ marginTop: "1.25rem" }}>
            <h4 style={{ margin: "0 0 0.75rem 0" }}>
              DAG Execution Nodes ({definition.nodes.length})
            </h4>

            {definition.nodes.length === 0 ? (
              <p>No nodes defined in this workflow template.</p>
            ) : (
              <div className="surface-table-wrap">
                <table className="surface-table" aria-label="Workflow nodes">
                  <thead>
                    <tr>
                      <th scope="col">Node Key</th>
                      <th scope="col">Title</th>
                      <th scope="col">Role</th>
                      <th scope="col">Permission Profile</th>
                      <th scope="col">Dependencies</th>
                      <th scope="col">Timeout</th>
                      <th scope="col">Approval</th>
                    </tr>
                  </thead>
                  <tbody>
                    {definition.nodes.map((node: WorkflowNode) => (
                      <tr key={node.key} data-testid={`node-row-${node.key}`}>
                        <td className="surface-mono">{node.key}</td>
                        <td>{node.title}</td>
                        <td>
                          {typeof node.role === "string"
                            ? node.role
                            : `custom: ${node.role.custom}`}
                        </td>
                        <td>
                          <span className="surface-mono">{node.permission_profile_hint}</span>
                        </td>
                        <td>
                          {node.depends_on.length > 0 ? (
                            <span className="surface-mono">{node.depends_on.join(", ")}</span>
                          ) : (
                            <span style={{ opacity: 0.6 }}>— (Root)</span>
                          )}
                        </td>
                        <td>{node.timeout_secs ? `${node.timeout_secs}s` : "Default"}</td>
                        <td>{node.requires_approval ? "Required" : "Autonomous"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </article>
      )}
    </section>
  );
}
