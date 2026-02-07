import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import { workspace, ExtensionContext, window, Uri } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

function findSnowc(): string | undefined {
  // 1. Explicit user setting (non-default)
  const config = workspace.getConfiguration("snow.lsp");
  const configured = config.get<string>("path");
  if (configured && configured !== "snowc") {
    if (fs.existsSync(configured)) {
      return configured;
    }
  }

  // 2. Workspace-local build (developing Snow itself, or local install)
  const workspaceFolders = workspace.workspaceFolders;
  if (workspaceFolders) {
    for (const folder of workspaceFolders) {
      const candidates = [
        path.join(folder.uri.fsPath, "target", "debug", "snowc"),
        path.join(folder.uri.fsPath, "target", "release", "snowc"),
      ];
      for (const c of candidates) {
        if (fs.existsSync(c)) {
          return c;
        }
      }
    }
  }

  // 3. Well-known install locations
  const home = os.homedir();
  const wellKnown = [
    path.join(home, ".snow", "bin", "snowc"),
    "/usr/local/bin/snowc",
    "/opt/homebrew/bin/snowc",
  ];
  for (const p of wellKnown) {
    if (fs.existsSync(p)) {
      return p;
    }
  }

  // 4. Fall back to PATH lookup (let the OS resolve it)
  return "snowc";
}

async function startClient(snowcPath: string) {
  const serverOptions: ServerOptions = {
    command: snowcPath,
    args: ["lsp"],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "snow" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.snow"),
    },
  };

  client = new LanguageClient(
    "snow-lsp",
    "Snow Language Server",
    serverOptions,
    clientOptions
  );

  try {
    await client.start();
  } catch (err: any) {
    const action = await window.showErrorMessage(
      `Snow LSP failed to start. Could not find or run '${snowcPath}'. ` +
        `Install Snow or configure the path to snowc.`,
      "Configure Path",
      "Dismiss"
    );

    if (action === "Configure Path") {
      const uris = await window.showOpenDialog({
        canSelectFiles: true,
        canSelectFolders: false,
        canSelectMany: false,
        title: "Select the snowc binary",
        openLabel: "Select snowc",
      });

      if (uris && uris.length > 0) {
        const selected = uris[0].fsPath;
        await workspace
          .getConfiguration("snow.lsp")
          .update("path", selected, true);
        window.showInformationMessage(
          `Snow LSP path set to '${selected}'. Reload window to activate.`
        );
      }
    }
  }
}

export function activate(context: ExtensionContext) {
  const snowcPath = findSnowc();
  if (snowcPath) {
    startClient(snowcPath);
  }
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
