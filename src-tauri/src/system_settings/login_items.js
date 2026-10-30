// Fixed JXA program embedded by Rust. Arguments are data, never executable source.
// Only legacy/user-added Open at Login applications; not SMAppService/background approvals.
function canonical(items) {
    if (!Array.isArray(items) || items.length > 64) throw Error('Login item bound exceeded');
    var seen = {};
    return items
        .map(function (item) {
            if (
                typeof item.path !== 'string' ||
                item.path.length > 2048 ||
                item.path[0] !== '/' ||
                !item.path.endsWith('.app') ||
                /[\x00-\x1f]/.test(item.path) ||
                item.path.split('/').indexOf('..') !== -1 ||
                typeof item.hidden !== 'boolean'
            )
                throw Error('Not a supported application login item');
            if (seen[item.path]) throw Error('Duplicate login application identity');
            seen[item.path] = true;
            return { path: item.path, hidden: item.hidden };
        })
        .sort(function (a, b) {
            return a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
        });
}
function run(argv) {
    if (argv.length !== 1 && argv.length !== 3) throw Error('Invalid fixed login operation');
    var se = Application('System Events');
    function read() {
        return canonical(
            se.loginItems().map(function (item) {
                return { path: item.path(), hidden: item.hidden() };
            })
        );
    }
    var current = read();
    if (argv[0] === 'read' && argv.length === 1) return JSON.stringify(current);
    if (argv[0] !== 'write' || argv.length !== 3) throw Error('Unknown fixed login operation');
    var expected = canonical(JSON.parse(argv[1]));
    var target = canonical(JSON.parse(argv[2]));
    if (JSON.stringify(current) !== JSON.stringify(expected))
        throw Error('Login items changed; refresh preview');
    var live = expected;
    // Each mutation checks the entire expected inventory. No launch, quit, file deletion, or shell.
    current.forEach(function (old) {
        if (
            !target.some(function (t) {
                return t.path === old.path && t.hidden === old.hidden;
            })
        ) {
            if (JSON.stringify(read()) !== JSON.stringify(live))
                throw Error('Login items changed during restore');
            var handles = se.loginItems().filter(function (i) {
                return i.path() === old.path;
            });
            if (handles.length !== 1 || handles[0].hidden() !== old.hidden)
                throw Error('Login identity changed');
            handles[0].delete();
            live = live.filter(function (i) {
                return i.path !== old.path;
            });
            if (JSON.stringify(read()) !== JSON.stringify(live))
                throw Error('Login removal readback mismatch');
        }
    });
    target.forEach(function (item) {
        if (
            !live.some(function (i) {
                return i.path === item.path && i.hidden === item.hidden;
            })
        ) {
            if (JSON.stringify(read()) !== JSON.stringify(live))
                throw Error('Login items changed during restore');
            se.loginItems.push(se.LoginItem({ path: item.path, hidden: item.hidden }));
            live = canonical(live.concat([item]));
            if (JSON.stringify(read()) !== JSON.stringify(live))
                throw Error('Login addition readback mismatch');
        }
    });
    var actual = read();
    if (JSON.stringify(actual) !== JSON.stringify(target))
        throw Error('Login inventory readback mismatch');
    return JSON.stringify(actual);
}
