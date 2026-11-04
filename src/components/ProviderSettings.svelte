<script lang="ts">
    import '../settings.css';
    import Icon from './Icon.svelte';
    import notices from '../../THIRD_PARTY_NOTICES.md?raw';
    import AutomationSettings from './AutomationSettings.svelte';
    import SettingsDefaults from './SettingsDefaults.svelte';
    import { untrack } from 'svelte';
    import type { Bootstrap } from '../lib/types';

    type Section = 'assistance' | 'integrations' | 'system';
    let {
        bootstrap,
        desktop,
        onrefresh,
        initialSection = 'assistance',
    }: {
        bootstrap: Bootstrap;
        desktop: boolean;
        onrefresh: () => Promise<void>;
        initialSection?: Section;
    } = $props();
    const sections = [
        { id: 'assistance', label: 'Assistance', icon: 'activity' },
        { id: 'integrations', label: 'Integrations', icon: 'code' },
        { id: 'system', label: 'System', icon: 'settings' },
    ] as const;
    let section: Section = $state(untrack(() => initialSection));
    function chooseSection(next: Section) {
        section = next;
        document.getElementById('settings-content')?.closest('main')?.scrollTo({ top: 0 });
    }
    function navigate(event: KeyboardEvent) {
        const current = sections.findIndex((item) => item.id === section);
        let index = current;
        if (event.key === 'ArrowRight') index = (current + 1) % sections.length;
        else if (event.key === 'ArrowLeft')
            index = (current + sections.length - 1) % sections.length;
        else if (event.key === 'Home') index = 0;
        else if (event.key === 'End') index = sections.length - 1;
        else return;
        event.preventDefault();
        chooseSection(sections[index].id);
        document.getElementById(`settings-tab-${section}`)?.focus();
    }
</script>

<div class="settings-workspace">
    <div class="settings-title">
        <h1>Settings</h1>
        {#if !desktop}<span class="tag">Preview · changes stay here</span>{/if}
    </div>
    <div class="settings-tabs" role="tablist" aria-label="Settings categories">
        {#each sections as item}
            <button
                id={`settings-tab-${item.id}`}
                role="tab"
                aria-selected={section === item.id}
                aria-controls="settings-content"
                tabindex={section === item.id ? 0 : -1}
                onclick={() => chooseSection(item.id)}
                onkeydown={navigate}><Icon name={item.icon} size={17} />{item.label}</button
            >
        {/each}
    </div>
    <div
        id="settings-content"
        role="tabpanel"
        aria-labelledby={`settings-tab-${section}`}
        tabindex="0"
    >
        <AutomationSettings
            {bootstrap}
            {desktop}
            {onrefresh}
            section={section === 'integrations' ? 'connections' : 'assistance'}
            visible={section === 'assistance' || section === 'integrations'}
        />
        <div hidden={section !== 'system'}>
            <SettingsDefaults {desktop} />
            <details class="preference-detail">
                <summary
                    ><span class="detail-name"
                        ><Icon name="info" /> Open-source acknowledgments</span
                    ></summary
                >
                <div class="detail-body"><pre class="license-text">{notices}</pre></div>
            </details>
        </div>
    </div>
</div>
