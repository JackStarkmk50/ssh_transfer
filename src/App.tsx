import { useState } from "react";
import {
  ChevronRight, Folder, FolderOpen, HardDrive, Home, KeyRound, MoreHorizontal,
  PanelLeft, Plus, RefreshCw, Search, Settings2, Upload, X
} from "lucide-react";

type Item = { name: string; kind: "folder" | "file"; detail: string; modified: string };

const localItems: Item[] = [
  { name: "Projects", kind: "folder", detail: "12 items", modified: "Today" },
  { name: "Archive", kind: "folder", detail: "4 items", modified: "Yesterday" },
  { name: "release-notes.pdf", kind: "file", detail: "2.4 MB", modified: "Oct 21" },
  { name: "site-build.zip", kind: "file", detail: "184 MB", modified: "Oct 19" }
];
const remoteItems: Item[] = [
  { name: "backups", kind: "folder", detail: "8 items", modified: "Today" },
  { name: "incoming", kind: "folder", detail: "Empty", modified: "Oct 20" },
  { name: "deploy", kind: "folder", detail: "3 items", modified: "Oct 18" },
  { name: "server.log", kind: "file", detail: "824 KB", modified: "Oct 24" }
];

function FileRow({ item }: { item: Item }) {
  return <button className="file-row">
    {item.kind === "folder" ? <Folder className="folder-icon" size={18} /> : <div className="file-icon">{item.name.split(".").pop()}</div>}
    <span className="file-name">{item.name}</span>
    <span className="file-detail">{item.detail}</span>
    <span className="file-date">{item.modified}</span>
    <MoreHorizontal className="row-menu" size={18} />
  </button>;
}

function FilePane({ remote }: { remote?: boolean }) {
  const items = remote ? remoteItems : localItems;
  return <section className="file-pane">
    <div className="pane-heading">
      <div>
        <p>{remote ? "Remote" : "This computer"}</p>
        <strong>{remote ? "lab-server" : "Rajendran"}</strong>
      </div>
      <button className="icon-button" aria-label="More options"><MoreHorizontal size={20} /></button>
    </div>
    <div className="crumbs">
      <Home size={15} /><ChevronRight size={14} />
      <span>{remote ? "/home/rajendran" : "C:\\Users\\Rajendran"}</span>
    </div>
    <div className="file-header"><span>Name</span><span>Size</span><span>Modified</span></div>
    <div className="file-list">{items.map((item) => <FileRow key={item.name} item={item} />)}</div>
    <div className="pane-footer">{items.length} items <span>•</span> {remote ? "SFTP" : "Local drive"}</div>
  </section>;
}

export default function App() {
  const [connected, setConnected] = useState(false);
  const [transferring, setTransferring] = useState(false);
  const [progress, setProgress] = useState(0);
  const connect = () => setConnected((value) => !value);
  const transfer = () => {
    setTransferring(true); setProgress(8);
    const tick = window.setInterval(() => {
      setProgress((current) => {
        if (current >= 100) { window.clearInterval(tick); setTransferring(false); return 100; }
        return Math.min(current + 14, 100);
      });
    }, 250);
  };
  return <main className="app-shell">
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark"><FolderOpen size={19} /></div><span>Shelf</span></div>
      <nav>
        <button className="nav-item active"><HardDrive size={18} /> Transfers</button>
        <button className="nav-item"><KeyRound size={18} /> Connections</button>
      </nav>
      <div className="connection-card">
        <div className="server-dot" />
        <div><strong>{connected ? "lab-server" : "Not connected"}</strong><span>{connected ? "sftp.example.net" : "Choose a connection"}</span></div>
        <button className="icon-button"><Settings2 size={16} /></button>
      </div>
      <button className="new-connection"><Plus size={17} /> New connection</button>
    </aside>
    <div className="work-area">
      <header className="topbar">
        <div><h1>File transfer</h1><p>Move files between this computer and an SSH server.</p></div>
        <div className="top-actions">
          <button className="quiet-button"><RefreshCw size={16} /> Refresh</button>
          <button className={connected ? "connection-button connected" : "connection-button"} onClick={connect}>{connected ? "Connected" : "Connect"}</button>
        </div>
      </header>
      <div className="transfer-toolbar">
        <button className="search"><Search size={17} /><span>Search files</span></button>
        <div className="transfer-choices">
          <button className="transfer-button" onClick={transfer} disabled={!connected}><Upload size={17} /> Upload to server</button>
          <button className="icon-button bordered" aria-label="Upload options"><ChevronRight size={16} /></button>
        </div>
      </div>
      <div className="panes"><FilePane /><div className="pane-divider" /><FilePane remote /></div>
      {(transferring || progress === 100) && <section className="transfer-queue">
        <div className="queue-icon"><Upload size={18} /></div>
        <div className="queue-copy"><strong>{progress === 100 ? "Upload complete" : "Uploading site-build.zip"}</strong><span>{progress === 100 ? "184 MB sent to /home/rajendran/incoming" : `${progress}% · 12.8 MB/s · 1 min remaining`}</span><div className="progress-track"><i style={{ width: `${progress}%` }} /></div></div>
        <button className="icon-button" onClick={() => { setTransferring(false); setProgress(0); }} aria-label="Dismiss transfer"><X size={18} /></button>
      </section>}
    </div>
  </main>;
}
