// Which edition of ArcScan this is, and how it treats session state.
//
// The backend decides all of this and the interface only displays it. There is
// deliberately no path arithmetic here and no way to ask for a different answer:
// Installed builds report their normal data root. Portable builds deliberately
// do not expose the internal temporary path: the only path-oriented action they
// offer opens a fixed download URL, and exports use an operator-chosen location.

export type Edition = "installed" | "portable";
export type UpdaterMode = "installer" | "manual";
export type StorageMode = "persistent" | "temporary";

export interface RuntimeInfo {
  edition: Edition;
  version: string;
  /** "Windows", "macOS", "Linux" — the build's target, not the host's. */
  platform: string;
  /** "x64", "ARM64", "x86" — the build's target, not the host's. */
  architecture: string;
  storage_mode: StorageMode;
  /** Installed data root. Null for disposable Portable sessions. */
  data_root: string | null;
  updater_mode: UpdaterMode;
}

export const isPortable = (info: RuntimeInfo | null): boolean => info?.edition === "portable";

/**
 * How the edition is described in Settings: "Portable edition · Windows x64".
 *
 * Both words after the separator come from the backend, which reads them off its
 * own compile target. Neither comes from the user agent, because the point of
 * the line is to say which *build* is running, and a user agent describes the
 * machine — which is exactly the wrong answer for an x64 build running on an
 * ARM64 Windows box under emulation.
 */
export function editionLabel(info: RuntimeInfo): string {
  const edition = info.edition === "portable" ? "Portable edition" : "Installed edition";
  return `${edition} · ${info.platform} ${info.architecture}`;
}

/**
 * What to tell a portable operator when a newer version exists.
 *
 * Not "Update now". Exports are the only intentional persistence mechanism,
 * so the order matters: retain work first, end the disposable session, then
 * fetch a fresh ZIP.
 */
export const PORTABLE_UPDATE_STEPS =
  "Export anything you want to keep, finish this session and close ArcScan, then download and extract " +
  "the latest Portable ZIP.";
