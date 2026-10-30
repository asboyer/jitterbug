import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const script = new URL('../src-tauri/src/system_settings/login_items.js', import.meta.url);
function harness(initial, reject = false) {
    let state = structuredClone(initial);
    const se = {
        loginItems: Object.assign(
            () =>
                state.map((item) => ({
                    path: () => item.path,
                    hidden: () => item.hidden,
                    delete: () => {
                        if (reject) throw Error('denied');
                        state = state.filter((x) => x !== item);
                    },
                })),
            {
                push: (item) => {
                    if (reject) throw Error('denied');
                    state.push(item);
                },
            }
        ),
        LoginItem: (item) => structuredClone(item),
    };
    const context = vm.createContext({
        Application: (name) => {
            assert.equal(name, 'System Events');
            return se;
        },
    });
    vm.runInContext(readFileSync(script, 'utf8'), context);
    return { run: (args) => context.run(args), state: () => state };
}
test('captures paths and hidden flags, not names or executable commands', () => {
    const h = harness([{ path: '/Applications/Example.app', hidden: true }]);
    assert.deepEqual(JSON.parse(h.run(['read'])), h.state());
});
test('finite restore preserves unchanged items and exact hidden flags', () => {
    const old = [{ path: '/Applications/A.app', hidden: true }];
    const target = [
        { path: '/Applications/A.app', hidden: false },
        { path: '/Applications/B.app', hidden: true },
    ];
    const h = harness(old);
    assert.deepEqual(
        JSON.parse(h.run(['write', JSON.stringify(old), JSON.stringify(target)])),
        target
    );
});
test('stale state, duplicates, non-application paths and denial fail closed', () => {
    const old = [{ path: '/Applications/A.app', hidden: true }];
    assert.throws(() => harness([]).run(['write', JSON.stringify(old), '[]']), /changed/);
    assert.throws(() => harness([...old, ...old]).run(['read']), /Duplicate/);
    assert.throws(() => harness([{ path: '/bin/sh', hidden: false }]).run(['read']), /application/);
    assert.throws(() => harness(old, true).run(['write', JSON.stringify(old), '[]']), /denied/);
});
