import type { Mode, Provider, Settings, Tier } from './types';

export const modeChoices: { value: Mode; label: string; text: string; hint: string }[] = [
    {
        value: 'local_only',
        label: 'Local',
        text: 'Small models on this computer. No cloud calls.',
        hint: 'On this computer',
    },
    {
        value: 'hybrid',
        label: 'Hybrid',
        text: 'Local for quick help. Frontier for depth.',
        hint: 'Local + frontier',
    },
    {
        value: 'cloud_only',
        label: 'Frontier',
        text: 'Cloud models with your API key.',
        hint: 'Cloud, your key',
    },
];

export const modeLabel = (mode: Mode) =>
    modeChoices.find((choice) => choice.value === mode)?.label ?? mode;

export const providerNames: Record<Provider, string> = {
    ollama: 'Ollama',
    meta: 'Meta',
    anthropic: 'Claude',
    jev: 'Jev',
};

export const localModel: Tier = { provider: 'ollama', model: 'qwen3.5:0.8b' };
export const frontierModel: Tier = { provider: 'meta', model: 'muse-spark-1.3' };

/** Frontier models offered during setup; every ID has a reviewed price in settings.rs. */
export const frontierChoices: (Tier & { label: string; text: string; recommended?: boolean })[] = [
    {
        provider: 'meta',
        model: 'muse-wonder-1.0',
        label: 'Meta Muse Wonder 1.0',
        text: 'Meta’s reasoning model, for the hardest questions.',
    },
    {
        provider: 'meta',
        model: 'muse-spark-1.3',
        label: 'Meta Muse Spark 1.3',
        text: 'Strong reasoning for investigations.',
        recommended: true,
    },
    {
        provider: 'anthropic',
        model: 'claude-sonnet-5-5',
        label: 'Claude Sonnet 5.5',
        text: 'Detailed explanations.',
    },
    {
        provider: 'anthropic',
        model: 'claude-haiku-5-5',
        label: 'Claude Haiku 5.5',
        text: 'Fastest and cheapest frontier option.',
    },
];

const isLocal = (tier: Tier) => tier.provider === 'ollama';

/**
 * Switch modes and move each tier the new mode would block onto something it allows, so a switch
 * never leaves Jitterbug unable to answer. Tiers the mode still permits keep their choice.
 */
export function applyMode(settings: Settings, mode: Mode, cloud: Tier = frontierModel): Settings {
    const next = structuredClone(settings);
    next.mode = mode;
    const cloudTier = next.intelligent.provider !== 'ollama' ? next.intelligent : cloud;
    if (mode === 'local_only') {
        if (!isLocal(next.cheap)) next.cheap = { ...localModel };
        if (!isLocal(next.intelligent)) next.intelligent = { ...localModel };
        if (!isLocal(next.decision)) next.decision = { provider: 'ollama', model: 'tev1:0.8b' };
    } else if (mode === 'cloud_only') {
        if (isLocal(next.cheap)) next.cheap = { ...cloudTier };
        if (isLocal(next.intelligent)) next.intelligent = { ...cloudTier };
        if (isLocal(next.decision)) next.decision = { ...cloudTier };
    } else {
        if (!isLocal(next.cheap)) next.cheap = { ...localModel };
        if (isLocal(next.intelligent)) next.intelligent = { ...cloud };
    }
    return next;
}

/** Cloud providers this configuration calls, and which of them still need a key. */
export function missingKeys(settings: Settings, credentials: Provider[]): Provider[] {
    const tiers = [settings.cheap, settings.intelligent];
    if (settings.decision_enabled) tiers.push(settings.decision);
    const used = new Set(
        tiers.filter((tier) => tier.provider !== 'ollama').map((tier) => tier.provider)
    );
    if (settings.mode === 'local_only') return [];
    return [...used].filter((provider) => !credentials.includes(provider));
}
