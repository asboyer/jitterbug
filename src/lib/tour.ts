export const tourPreference = 'jitterbug.tour.v1.seen';

export const tourSteps = [
    {
        page: 'monitor',
        target: 'health',
        title: 'Start with how your computer is doing',
        body: 'CPU is work happening now; memory is room to work. High usage can be normal. Pressure tells you when the computer is struggling.',
    },
    {
        page: 'monitor',
        target: 'workloads',
        title: 'Find the app behind the activity',
        body: 'Expand an app to see its processes. Select one for an automatic AI investigation of that specific instance, then ask a follow-up.',
    },
    {
        page: 'inbox',
        target: 'proposal',
        title: 'A suggestion first. Your permission next.',
        body: 'Tap ⓘ for an explanation, or Review change for the effects and permission choices. Nothing runs just because you open a suggestion. View details holds the evidence.',
    },
    {
        page: 'agents',
        target: 'agents',
        title: 'See what every agent is doing',
        body: 'Each working Claude Code, Codex, or Muse Code conversation shows what it is doing now, your last request, and what it uses. Connected agents run tests and builds one at a time and keep working while they wait.',
    },
    {
        page: 'monitor',
        target: 'settings',
        title: 'You choose the boundaries',
        body: 'The AI page sets local or frontier models and spending limits; Settings holds automatic permissions and agent connections. Jitterbug never automatically stops processes. You can replay this tour from the sidebar.',
    },
] as const;
