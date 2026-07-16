const vscode = require('vscode');
const { exec } = require('child_process');
const path = require('path');
const fs = require('fs');

/**
 * Activates the L++ language support extension.
 * @param {vscode.ExtensionContext} context
 */
function activate(context) {
    console.log('L++ Language Support activated!');

    // Create a diagnostic collection for compiler errors
    const diagnosticCollection = vscode.languages.createDiagnosticCollection('lpp');
    context.subscriptions.push(diagnosticCollection);

    // Register listener for document events to run diagnostics
    context.subscriptions.push(
        vscode.workspace.onDidSaveTextDocument(doc => runCompilerDiagnostics(doc, diagnosticCollection)),
        vscode.workspace.onDidOpenTextDocument(doc => runCompilerDiagnostics(doc, diagnosticCollection)),
        vscode.workspace.onDidCloseTextDocument(doc => diagnosticCollection.delete(doc.uri))
    );

    // Run diagnostics on currently active editor if any
    if (vscode.window.activeTextEditor) {
        runCompilerDiagnostics(vscode.window.activeTextEditor.document, diagnosticCollection);
    }

    // 1. Auto Completion Provider
    const completionProvider = vscode.languages.registerCompletionItemProvider('lpp', {
        provideCompletionItems(document, position) {
            const items = [];

            // Keywords
            const keywords = ['def', 'struct', 'fn', 'return', 'spawn', 'if', 'else', 'while', 'mut'];
            keywords.forEach(kw => {
                const item = new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword);
                item.detail = 'L++ Keyword';
                items.push(item);
            });

            // Types
            const types = ['Int', 'String', 'Bool', 'Void'];
            types.forEach(t => {
                const item = new vscode.CompletionItem(t, vscode.CompletionItemKind.TypeParameter);
                item.detail = 'L++ Primitive Type';
                items.push(item);
            });

            // Built-ins
            const builtins = [
                { name: 'print', detail: 'print(value: Int | String) -> Void\nPrints value to console.' },
                { name: 'print_str', detail: 'print_str(value: String) -> Void\nPrints string to console.' },
                { name: 'input', detail: 'input() -> String\nReads a line of text from stdin.' },
                { name: 'read_file', detail: 'read_file(path: String) -> String\nReads the entire content of a file.' },
                { name: 'write_file', detail: 'write_file(path: String, data: String) -> Void\nWrites string data to a file.' }
            ];
            builtins.forEach(b => {
                const item = new vscode.CompletionItem(b.name, vscode.CompletionItemKind.Function);
                item.detail = 'L++ Built-in';
                item.documentation = new vscode.MarkdownString(b.detail);
                items.push(item);
            });

            // Dynamic completion scanning of active document
            const text = document.getText();
            const defRegex = /\bdef\s+(\w+)/g;
            const structRegex = /\bstruct\s+(\w+)/g;
            const varRegex = /\b(mut\s+)?(\w+)\s*:=/g;
            let match;

            // Find all custom functions
            while ((match = defRegex.exec(text)) !== null) {
                if (!keywords.includes(match[1]) && !builtins.some(b => b.name === match[1])) {
                    items.push(new vscode.CompletionItem(match[1], vscode.CompletionItemKind.Function));
                }
            }

            // Find all custom structs
            while ((match = structRegex.exec(text)) !== null) {
                if (!keywords.includes(match[1]) && !types.includes(match[1])) {
                    items.push(new vscode.CompletionItem(match[1], vscode.CompletionItemKind.Struct));
                }
            }

            // Find all local variables
            while ((match = varRegex.exec(text)) !== null) {
                const varName = match[2];
                if (!keywords.includes(varName) && !builtins.some(b => b.name === varName)) {
                    items.push(new vscode.CompletionItem(varName, vscode.CompletionItemKind.Variable));
                }
            }

            return items;
        }
    }, '.'); // Trigger on dot to support field completions conceptually
    context.subscriptions.push(completionProvider);

    // 2. Hover Provider (Includes Escape Analysis storage classes!)
    const hoverProvider = vscode.languages.registerHoverProvider('lpp', {
        async provideHover(document, position) {
            const range = document.getWordRangeAtPosition(position);
            if (!range) return null;
            const word = document.getText(range);

            // Built-in / Keyword documentation
            const docs = {
                'def': '**def**: Defines a function with explicit parameter types and return type. Enables rapid compile checking.',
                'struct': '**struct**: Defines a structured value type. Stored on Stack by default; promoted to Arenas if self-referential, or Managed Heap if it escapes.',
                'fn': '**fn**: Creates an anonymous inline closure or lambda function.',
                'mut': '**mut**: Marks a variable binding as mutable, allowing reassignment via `=` operator.',
                'spawn': '**spawn**: Launches a concurrent asynchronous task running the specified closure.',
                'return': '**return**: Returns a value from the current function frame.',
                'if': '**if**: Branches code execution based on a boolean condition.',
                'else': '**else**: Code block executed when `if` condition evaluates to false.',
                'while': '**while**: Iterates a block of code as long as a condition evaluates to true.',
                'print': '**print(value)**: Built-in console logger. Handles integer values and formats them.',
                'print_str': '**print_str(string)**: Outputs raw string literal to console.',
                'input': '**input()**: Blocking standard input call. Reads next line from console.',
                'read_file': '**read_file(path)**: Reads entire contents of the specified file path.',
                'write_file': '**write_file(path, content)**: Writes contents to the specified file path.'
            };

            if (docs[word]) {
                return new vscode.Hover(new vscode.MarkdownString(docs[word]));
            }

            // Read compile output mapping to extract escape classification!
            try {
                const storageClass = await queryCompilerStorageClass(document, word);
                if (storageClass) {
                    const md = new vscode.MarkdownString();
                    md.appendMarkdown(`### L++ Semantic Analyzer\n\n`);
                    md.appendMarkdown(`* **Identifier**: \`${word}\`\n`);
                    md.appendMarkdown(`* **Storage Class**: \`${storageClass}\`\n\n`);
                    
                    if (storageClass === 'Value') {
                        md.appendMarkdown(`> **Optimized Stack Allocation**: This variable has zero allocation cost, no GC/ARC runtime overhead, and is automatically cleaned up when the stack frame ends.`);
                    } else if (storageClass === 'Arc') {
                        md.appendMarkdown(`> **Managed Heap (Atomic RC)**: This variable escapes its stack frame or crosses a thread/concurrency boundary, so the compiler automatically promotes it to ARC.`);
                    } else if (storageClass === 'Arena') {
                        md.appendMarkdown(`> **Arena Region Allocation**: This self-referential/recursive structure is bump-allocated in a high-performance memory region.`);
                    }
                    return new vscode.Hover(md);
                }
            } catch (e) {
                // Ignore compiler querying errors silently
            }

            return null;
        }
    });
    context.subscriptions.push(hoverProvider);

    // 3. Go to Definition Provider
    const definitionProvider = vscode.languages.registerDefinitionProvider('lpp', {
        provideDefinition(document, position) {
            const range = document.getWordRangeAtPosition(position);
            if (!range) return null;
            const word = document.getText(range);

            const text = document.getText();
            const lines = text.split('\n');

            // Look for def function, struct name, or variable binding
            const searchRegexes = [
                new RegExp(`\\bdef\\s+${word}\\b`),
                new RegExp(`\\bstruct\\s+${word}\\b`),
                new RegExp(`\\b(mut\\s+)?${word}\\s*:=`)
            ];

            for (let i = 0; i < lines.length; i++) {
                for (const regex of searchRegexes) {
                    if (regex.test(lines[i])) {
                        const col = lines[i].indexOf(word);
                        return new vscode.Location(
                            document.uri,
                            new vscode.Position(i, col >= 0 ? col : 0)
                        );
                    }
                }
            }

            return null;
        }
    });
    context.subscriptions.push(definitionProvider);

    // 4. Outline/Document Symbols Provider
    const symbolProvider = vscode.languages.registerDocumentSymbolProvider('lpp', {
        provideDocumentSymbols(document) {
            const symbols = [];
            const text = document.getText();
            const lines = text.split('\n');

            const defRegex = /^\s*def\s+(\w+)\s*\(/;
            const structRegex = /^\s*struct\s+(\w+)\s*:/;

            for (let i = 0; i < lines.length; i++) {
                let match = defRegex.exec(lines[i]);
                if (match) {
                    const name = match[1];
                    const symbolRange = new vscode.Range(i, 0, i, lines[i].length);
                    symbols.push(new vscode.DocumentSymbol(
                        name,
                        'Function Definition',
                        vscode.SymbolKind.Function,
                        symbolRange,
                        symbolRange
                    ));
                    continue;
                }

                match = structRegex.exec(lines[i]);
                if (match) {
                    const name = match[1];
                    const symbolRange = new vscode.Range(i, 0, i, lines[i].length);
                    symbols.push(new vscode.DocumentSymbol(
                        name,
                        'Struct Definition',
                        vscode.SymbolKind.Struct,
                        symbolRange,
                        symbolRange
                    ));
                }
            }

            return symbols;
        }
    });
    context.subscriptions.push(symbolProvider);
}

/**
 * Searches the background L++ compilation storage classification map
 * to find the storage allocation category of a variable.
 * @param {vscode.TextDocument} document
 * @param {string} varName
 * @returns {Promise<string|null>}
 */
function queryCompilerStorageClass(document, varName) {
    return new Promise((resolve) => {
        const filePath = document.fileName;
        const projectDir = path.dirname(filePath);
        // Find lpp.exe in build path
        // We look for release executable
        const lppBin = path.join(vscode.workspace.rootPath || '', 'target', 'release', 'lpp.exe');
        if (!fs.existsSync(lppBin)) {
            return resolve(null);
        }

        // Run lpp.exe to extract map
        exec(`"${lppBin}" "${filePath}"`, { cwd: vscode.workspace.rootPath }, (err, stdout, stderr) => {
            if (stdout) {
                // Find "Binding '<varName>' -> <StorageClass>" in stdout
                const regex = new RegExp(`Binding\\s+'${varName}'\\s+->\\s+(\\w+)`);
                const match = regex.exec(stdout);
                if (match) {
                    return resolve(match[1]); // e.g. "Value", "Arc", "Arena"
                }
            }
            resolve(null);
        });
    });
}

/**
 * Runs the compiler in the background and sets diagnostic underlines for compilation errors.
 * @param {vscode.TextDocument} document
 * @param {vscode.DiagnosticCollection} collection
 */
function runCompilerDiagnostics(document, collection) {
    if (document.languageId !== 'lpp') return;

    const filePath = document.fileName;
    const lppBin = path.join(vscode.workspace.rootPath || '', 'target', 'release', 'lpp.exe');
    if (!fs.existsSync(lppBin)) return;

    exec(`"${lppBin}" "${filePath}"`, { cwd: vscode.workspace.rootPath }, (err, stdout, stderr) => {
        // Clear old diagnostics
        collection.delete(document.uri);

        const diagnostics = [];
        const lines = document.getText().split('\n');

        // Look for errors in stderr or stdout
        const output = (stdout || '') + '\n' + (stderr || '');
        const errorRegex = /(Lexer error|Parser error|Semantic error|Type check error|Escape Analysis error):\s*(.*)/g;
        let match;

        while ((match = errorRegex.exec(output)) !== null) {
            const errType = match[1];
            const errMsg = match[2];

            // Heuristic to locate target line
            let targetLine = 0;
            let targetChar = 0;
            let targetLength = 1;

            // If error specifies a variable name or function name, search for it
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

            // Create diagnostic entry
            const range = new vscode.Range(targetLine, targetChar, targetLine, targetChar + targetLength);
            const diag = new vscode.Diagnostic(
                range,
                `${errType}: ${errMsg}`,
                vscode.DiagnosticSeverity.Error
            );
            diagnostics.push(diag);
        }

        if (diagnostics.length > 0) {
            collection.set(document.uri, diagnostics);
        }
    });
}

function deactivate() {
    console.log('L++ Language Support deactivated.');
}

module.exports = {
    activate,
    deactivate
};
