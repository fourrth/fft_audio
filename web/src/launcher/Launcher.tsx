import { useEffect, useState, type DragEvent } from "react";
import { Chip } from "../ui/Chip";
import { IconButton } from "../ui/IconButton";
import { Tile } from "../ui/Tile";
import { ViewportMenu } from "../ui/ViewportMenu";
import {
  USAGE_TIPS,
  fileName,
  folderOf,
  isSpawnableTile,
  launcherTiles,
  nextTipIndex,
  tipAt,
} from "./recent";
import "./launcher.css";

export interface LauncherProps {
  recentFiles: string[];
  scriptingEnabled: boolean;
  onOpenSpectrogram: () => void;
  onOpenInSplit?: () => void;
  onOpenRecorder: () => void;
  onOpenScripting: () => void;
  onOpenFile: (path: string) => void;
}

type DroppedFile = File & { path?: string };

function eventHasFiles(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes("Files");
}

/**
 * New-tab page: root displays only, recent files, rotating tip and the
 * drag-and-drop affordance.
 */
export function Launcher({
  recentFiles,
  scriptingEnabled,
  onOpenSpectrogram,
  onOpenInSplit,
  onOpenRecorder,
  onOpenScripting,
  onOpenFile,
}: LauncherProps) {
  const [dragActive, setDragActive] = useState(false);
  const [tipIndex, setTipIndex] = useState(0);
  const [tileMenu, setTileMenu] = useState<{ x: number; y: number } | null>(
    null,
  );

  useEffect(() => {
    const timer = window.setInterval(
      () => setTipIndex((index) => nextTipIndex(index)),
      12_000,
    );
    return () => window.clearInterval(timer);
  }, []);

  const onDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (eventHasFiles(event)) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      setDragActive(true);
    }
  };

  const onDragLeave = (event: DragEvent<HTMLDivElement>) => {
    if (
      event.relatedTarget instanceof Node &&
      event.currentTarget.contains(event.relatedTarget)
    ) {
      return;
    }
    setDragActive(false);
  };

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragActive(false);
    const file = event.dataTransfer?.files?.[0] as DroppedFile | undefined;
    if (file) {
      onOpenFile(file.path || file.name);
    }
  };

  return (
    <div
      className="launcher"
      data-testid="launcher"
      data-ui-id="launcher"
      onDragEnter={onDragOver}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="launcher-inner">
        <header className="launcher-head">
          <div>
            <h1>Launcher</h1>
            <p>
              Open a root display. Order Cuts and Line Cuts live inside a
              Spectrogram, not here.
            </p>
          </div>
          <Chip tone="launcher">New tab</Chip>
        </header>

        <div className="launcher-tiles" aria-label="Root displays">
          {launcherTiles(scriptingEnabled).map((tile) => (
            <Tile
              key={tile.id}
              icon={tile.icon}
              title={tile.title}
              description={tile.description}
              kind={tile.kind}
              badge={tile.soon ? "Soon" : undefined}
              disabled={!isSpawnableTile(tile)}
              onClick={
                tile.id === "spectrogram"
                  ? onOpenSpectrogram
                  : tile.id === "recorder"
                    ? onOpenRecorder
                    : onOpenScripting
              }
              onContextMenu={
                tile.id === "spectrogram"
                  ? (event) => {
                      event.preventDefault();
                      setTileMenu({ x: event.clientX, y: event.clientY });
                    }
                  : undefined
              }
            />
          ))}
        </div>

        {tileMenu ? (
          <ViewportMenu
            className="tab-context-menu"
            role="menu"
            aria-label="Launcher tile actions"
            x={tileMenu.x}
            y={tileMenu.y}
            onClose={() => setTileMenu(null)}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setTileMenu(null);
                onOpenSpectrogram();
              }}
            >
              Open
            </button>
            <button
              type="button"
              role="menuitem"
              disabled={!onOpenInSplit}
              onClick={() => {
                setTileMenu(null);
                onOpenInSplit?.();
              }}
            >
              Open in new split
            </button>
          </ViewportMenu>
        ) : null}

        <section className="launcher-section" aria-labelledby="recent-heading">
          <div className="launcher-section-head">
            <h2 id="recent-heading">Recent files</h2>
            <span className="launcher-count">
              {recentFiles.length}/8
            </span>
          </div>
          {recentFiles.length === 0 ? (
            <p className="launcher-muted">
              No recent files yet — open a file or drop one here.
            </p>
          ) : (
            <ul className="recent-list">
              {recentFiles.map((path) => (
                <li key={path}>
                  <button
                    type="button"
                    className="recent-item"
                    title={path}
                    onClick={() => onOpenFile(path)}
                  >
                    <span className="recent-name">{fileName(path)}</span>
                    <span className="recent-folder">{folderOf(path) || "—"}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <div className="launcher-tip" role="note" aria-label="Usage tip">
          <span className="launcher-tip-mark" aria-hidden="true">
            ✦
          </span>
          <span className="launcher-tip-text">{tipAt(tipIndex)}</span>
          <IconButton
            label="Next tip"
            onClick={() => setTipIndex((index) => nextTipIndex(index))}
          >
            ↻
          </IconButton>
          <span className="launcher-tip-count">
            {tipIndex + 1}/{USAGE_TIPS.length}
          </span>
        </div>
      </div>

      {dragActive ? (
        <div className="launcher-drop" data-testid="launcher-drop-overlay">
          <div className="launcher-drop-card">
            <span className="launcher-drop-icon" aria-hidden="true">
              ⤓
            </span>
            <strong>Drop a file to open in a new Spectrogram</strong>
            <span>WAV, FLAC, MP3, OGG, AAC, or a CSV/Parquet table</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}
