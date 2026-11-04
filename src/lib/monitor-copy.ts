import type { ProcessRecord, Workload } from './types';

export function processRole(process: ProcessRecord, app: string): string {
    const name = process.name.toLowerCase();
    if (name === 'plugin-container' && /firefox/i.test(app))
        return 'Runs Firefox web content or a browser service in a separate process.';
    if (/renderer|webcontent/.test(name)) return 'Renders pages and the app’s interface.';
    if (/gpu/.test(name)) return 'Draws graphics and accelerates video.';
    if (/network/.test(name)) return 'Handles the app’s network connections.';
    if (/extension.?host/.test(name)) return 'Runs editor extensions.';
    if (/crashpad|crash.?handler/.test(name)) return 'Collects crash reports for the app.';
    if (/helper/.test(name)) return `Supports ${app} in the background.`;
    if (/^(zsh|bash|fish|sh)$/.test(name)) return 'Runs commands in a terminal session.';
    if (/^(node|bun|deno)$/.test(name)) return 'Runs JavaScript for a tool or development server.';
    if (/^python/.test(name)) return 'Runs a Python program.';
    if (/^rustc|clang|gcc/.test(name)) return 'Compiles source code into a program.';
    return process.name === app ? 'Main application process.' : `Runs as part of ${app}.`;
}
export function appFallback(workload: Workload, demo: boolean): string {
    if (demo) {
        return (
            {
                'demo-browser': 'Browses the web, with separate processes for tabs and extensions.',
                'demo-editor': 'Edits code and runs extensions and terminal sessions.',
                'demo-project': 'Runs the development server and build tools for this project.',
                'demo-model': 'Runs a small language model on this computer.',
            }[workload.id] ?? workload.description
        );
    }
    if (workload.category === 'project')
        return 'Development tools running from the same project folder.';
    if (workload.category === 'model') return 'Runs language models on this computer.';
    return 'Identifying this app…';
}
