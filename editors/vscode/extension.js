const vscode = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');
const { exec } = require('child_process');
const path = require('path');
const fs = require('fs');

let client;

/**
 * Resolves the L++ LSP language server executable dynamically.
 * @returns {string|null}
 */
function findLppLspBinary() {
    const configPath = vscode.workspace.getConfiguration('lpp').get('lspPath');
    if (configPath && fs.existsSync(configPath)) {
        return configPath;
    }

    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (workspaceFolders) {
        for (const folder of workspaceFolders) {
            const rootPath = folder.uri.fsPath;
            const debugLsp = path.join(rootPath, 'target', 'debug', process.platform === 'win32' ? 'lpp-lsp.exe' : 'lpp-lsp');
            if (fs.existsSync(debugLsp)) return debugLsp;

            const releaseLsp = path.join(rootPath, 'target', 'release', process.platform === 'win32' ? 'lpp-lsp.exe' : 'lpp-lsp');
            if (fs.existsSync(releaseLsp)) return releaseLsp;
        }
    }

    return process.platform === 'win32' ? 'lpp-lsp.exe' : 'lpp-lsp';
}

/**
 * Resolves the main L++ compiler binary.
 * @returns {string|null}
 */
function findLppBinary() {
    const configPath = vscode.workspace.getConfiguration('lpp').get('compilerPath');
    if (configPath && fs.existsSync(configPath)) {
        return configPath;
    }

    if (process.env.LPP_BIN && fs.existsSync(process.env.LPP_BIN)) {
        return process.env.LPP_BIN;
    }

    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (workspaceFolders) {
        for (const folder of workspaceFolders) {
            const rootPath = folder.uri.fsPath;
            const debugBin = path.join(rootPath, 'target', 'debug', process.platform === 'win32' ? 'lpp.exe' : 'lpp');
            if (fs.existsSync(debugBin)) return debugBin;

            const releaseBin = path.join(rootPath, 'target', 'release', process.platform === 'win32' ? 'lpp.exe' : 'lpp');
            if (fs.existsSync(releaseBin)) return releaseBin;
        }
    }

    return process.platform === 'win32' ? 'lpp.exe' : 'lpp';
}

/**
 * Activates the L++ extension with Language Server protocol support.
 * @param {vscode.ExtensionContext} context
 */
function activate(context) {
    console.log('L++ Language Support activating (Persistent LSP Architecture)...');

    const lspBin = findLppLspBinary();

    // 1. Try launching persistent native lpp-lsp server over stdio
    if (lspBin) {
        const serverOptions = {
            run: { command: lspBin, transport: TransportKind.stdio },
            debug: { command: lspBin, transport: TransportKind.stdio }
        };

        const clientOptions = {
            documentSelector: [{ scheme: 'file', language: 'lpp' }],
            synchronize: {
                fileEvents: vscode.workspace.createFileSystemWatcher('**/*.lpp')
            }
        };

        try {
            client = new LanguageClient(
                'lppLanguageServer',
                'L++ Language Server',
                serverOptions,
                clientOptions
            );
            client.start();
            console.log('L++ Language Server (lpp-lsp) started successfully.');
        } catch (e) {
            console.warn('Failed to start native lpp-lsp process, falling back to JS providers:', e);
            setupFallbackProviders(context);
        }
    } else {
        setupFallbackProviders(context);
    }

    // 2. Code Runner Integration
    try {
        const codeRunnerConfig = vscode.workspace.getConfiguration('code-runner');
        const executorMap = codeRunnerConfig.get('executorMap') || {};
        if (!executorMap.lpp) {
            const lppBin = findLppBinary() || 'lpp';
            executorMap.lpp = `"${lppBin}" run $fileName`;
            codeRunnerConfig.update('executorMap', executorMap, vscode.ConfigurationTarget.Global);
        }
    } catch (e) {}
}

/**
 * In-memory fallback providers when lpp-lsp binary is building or unavailable.
 */
function setupFallbackProviders(context) {
    const diagnosticCollection = vscode.languages.createDiagnosticCollection('lpp');
    context.subscriptions.push(diagnosticCollection);

    context.subscriptions.push(
        vscode.workspace.onDidSaveTextDocument(doc => runCompilerDiagnostics(doc, diagnosticCollection)),
        vscode.workspace.onDidOpenTextDocument(doc => runCompilerDiagnostics(doc, diagnosticCollection)),
        vscode.workspace.onDidCloseTextDocument(doc => diagnosticCollection.delete(doc.uri))
    );

    if (vscode.window.activeTextEditor) {
        runCompilerDiagnostics(vscode.window.activeTextEditor.document, diagnosticCollection);
    }
}

/**
 * Background diagnostics (in-memory --check, zero binary output).
 */
function runCompilerDiagnostics(document, collection) {
    if (document.languageId !== 'lpp') return;

    const lppBin = findLppBinary();
    if (!lppBin) return;

    exec(`"${lppBin}" run --check "${document.fileName}"`, (err, stdout, stderr) => {
        collection.delete(document.uri);

        const diagnostics = [];
        const lines = document.getText().split('\n');
        const output = (stdout || '') + '\n' + (stderr || '');
        const errorRegex = /(Lexer error|Parser error|Semantic error|Type check error|Escape Analysis error):\s*(.*)/g;
        let match;

        while ((match = errorRegex.exec(output)) !== null) {
            const errType = match[1];
            const errMsg = match[2];

            let targetLine = 0;
            let targetChar = 0;
            let targetLength = 1;

            const tokenMatch = /'([^']+)'/.exec(errMsg);
            if (tokenMatch) {
                const token = tokenMatch[1];
                for (let i = 0; i < lines.length; i++) {
                    const col = lines[i].indexOf(token);
                    if (col >= 0) {
                        targetLine = i;
                        targetChar = col;
                        targetLength = token.length;
                        break;
                    }
                }
            }

            const range = new vscode.Range(targetLine, targetChar, targetLine, targetChar + targetLength);
            diagnostics.push(new vscode.Diagnostic(
                range,
                `${errType}: ${errMsg}`,
                vscode.DiagnosticSeverity.Error
            ));
        }

        if (diagnostics.length > 0) {
            collection.set(document.uri, diagnostics);
        }
    });
}

function deactivate() {
    if (client) {
        return client.stop();
    }
}

module.exports = {
    activate,
    deactivate
};
