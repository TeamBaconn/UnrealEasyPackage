import { Component, type ReactNode } from "react";
import { tokens } from "./tokens";

/**
 * Catches render/runtime errors in the tree below it and shows the message + stack
 * instead of a blank white screen (React unmounts the whole tree on an uncaught
 * error). Each Tauri window wraps its surface in one so a thrown error is visible
 * and reportable, not silent.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: unknown) {
    // Surfaces in the devtools console too.
    console.error("UI error:", error, info);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        style={{
          minHeight: "100vh",
          background: tokens.page,
          padding: 28,
          display: "flex",
          flexDirection: "column",
          gap: 12,
          overflow: "auto",
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 700, color: tokens.ink }}>Something broke in the UI</div>
        <div style={{ fontSize: 13, color: tokens.danger, fontFamily: "var(--mantine-font-family-monospace)" }}>
          {error.message || String(error)}
        </div>
        {error.stack && (
          <pre
            style={{
              fontSize: 11,
              color: tokens.textMuted,
              background: tokens.surfaceAlt,
              border: `1px solid ${tokens.divider}`,
              borderRadius: 8,
              padding: 12,
              overflow: "auto",
              whiteSpace: "pre-wrap",
              maxHeight: 280,
            }}
          >
            {error.stack}
          </pre>
        )}
        <button
          onClick={() => this.setState({ error: null })}
          style={{
            alignSelf: "flex-start",
            padding: "8px 16px",
            borderRadius: 8,
            border: "none",
            background: tokens.accent,
            color: tokens.onAccent,
            fontSize: 13,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          Try again
        </button>
      </div>
    );
  }
}
