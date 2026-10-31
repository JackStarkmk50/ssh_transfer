import { useEffect, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

export type TerminalTab = { key: string; label: string };

function TerminalViewport({ tab, active }: { tab: TerminalTab; active: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const terminalId = useRef<number | undefined>(undefined);
  const fit = useRef<FitAddon | undefined>(undefined);
  const terminal = useRef<Terminal | undefined>(undefined);

  const resize = () => {
    try { fit.current?.fit(); } catch { return; }
    if (terminalId.current !== undefined && terminal.current) void invoke("resize_terminal", { terminalId: terminalId.current, columns: terminal.current.cols, rows: terminal.current.rows });
  };

  useEffect(() => {
    if (!host.current) return;
    const instance = new Terminal({ cursorBlink: true, fontFamily: "Cascadia Mono, Consolas, monospace", fontSize: 13, scrollback: 4000, theme: { background: "#101d26", foreground: "#d9e7e8", cursor: "#9de0bb", selectionBackground: "#315767" } });
    const addon = new FitAddon();
    terminal.current = instance;
    fit.current = addon;
    instance.loadAddon(addon);
    instance.open(host.current);
    resize();
    instance.writeln("Connecting remote terminal…");
    const output = new Channel<string>();
    output.onmessage = (data) => instance.write(data);
    void invoke<number>("open_terminal", { columns: instance.cols, rows: instance.rows, output }).then((id) => { terminalId.current = id; resize(); }).catch((error) => instance.writeln(`\r\nTerminal unavailable: ${String(error)}`));
    const input = instance.onData((data) => { if (terminalId.current !== undefined) void invoke("write_terminal", { terminalId: terminalId.current, data }); });
    const observer = new ResizeObserver(resize);
    observer.observe(host.current);
    return () => { input.dispose(); observer.disconnect(); if (terminalId.current !== undefined) void invoke("close_terminal", { terminalId: terminalId.current }); terminalId.current = undefined; instance.dispose(); terminal.current = undefined; fit.current = undefined; };
  }, [tab.key]);

  useEffect(() => { if (active) requestAnimationFrame(resize); }, [active]);

  return <div className={`terminal-viewport ${active ? "active" : ""}`}><div ref={host} className="terminal-host" /></div>;
}

export default function TerminalWorkspace({ tabs, activeKey }: { tabs: TerminalTab[]; activeKey?: string }) {
  return <section className="terminal-workspace"><header><div><h1>Remote terminals</h1><p>Commands run on the connected SSH server.</p></div><span>{tabs.length} open</span></header><div className="terminal-surface">{tabs.map((tab) => <TerminalViewport key={tab.key} tab={tab} active={tab.key === activeKey} />)}</div></section>;
}
