export const tourPreference = 'jitterbug.tour.v1.seen';

export const tourSteps = [
    {
        page: 'dashboard',
        target: 'health',
        title: 'Start with how your computer is doing',
        body: 'CPU is work happening now; memory is room to work. High usage can be normal. Pressure tells you when the computer is struggling.',
    },
    {
        page: 'workloads',
        target: 'workloads',
        title: 'Find the app behind the activity',
        body: 'Select an app to see its processes and where they came from. You can ask questions like “Why is Spotify using so much CPU?”',
    },
    {
        page: 'inbox',
        target: 'proposal',
        title: 'A suggestion first. Your permission next.',
        body: 'Tap ⓘ for an explanation, or Review change for the effects and permission choices. Nothing runs just because you open a suggestion. View details holds the evidence.',
    },
    {
        page: 'agents',
        target: 'agent-tasks',
        title: 'See what your coding tools are doing',
        body: 'Logos identify the harness. Open a task for its request, resource use, and suggestions. Green means the recorded session completed; it does not prove the work is correct.',
    },
    {
        page: 'queue',
        target: 'validation',
        title: 'Let heavy checks take turns',
        body: 'Manage a workload to approve a repeatable check. Connected tools can submit it here and wait for room. Only submitted checks are scheduled; other processes keep running.',
    },
    {
        page: 'dashboard',
        target: 'settings',
        title: 'You choose the boundaries',
        body: 'Settings controls local or cloud AI, spending limits, and automatic permissions. Jitterbug never automatically stops processes. You can replay this tour from the sidebar.',
    },
] as const;
