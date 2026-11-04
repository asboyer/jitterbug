<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import '../settings.css';
    import { errorMessage } from '../lib/format';
    import type { AgentClient } from '../lib/types';

    let {
        desktop,
        onconnect,
    }: {
        desktop: boolean;
        /** Called after a client connects; connecting also turns agent access on. */
        onconnect?: () => void | Promise<void>;
    } = $props();

    let clients = $state<AgentClient[] | null>(null);
    let busy = $state('');
    let error = $state('');

    const connected = (c: AgentClient) => c.mcp && c.hint !== false;

    onMount(() => {
        if (!desktop) {
            clients = [];
            return;
        }
        void invoke<AgentClient[]>('agent_clients')
            .then((found) => (clients = found))
            .catch((cause) => (error = errorMessage(cause)));
    });

    async function change(client: AgentClient, connect: boolean) {
        if (!desktop) return;
        busy = client.id;
        error = '';
        try {
            clients = await invoke<AgentClient[]>(
                connect ? 'connect_agent_client' : 'disconnect_agent_client',
                { id: client.id }
            );
            if (connect) await onconnect?.();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = '';
        }
    }

    function detail(client: AgentClient) {
        if (!client.installed) return 'Not found on this computer';
        if (!connected(client))
            return client.mcp ? 'Tools added; warnings not set up' : 'Not connected';
        return client.hint === null ? 'Tools added' : 'Tools added · warns before heavy work';
    }
</script>

<ul class="integration-list agent-clients" aria-label="Coding agents">
    {#each clients ?? [] as client (client.id)}
        <li>
            <div class="integration-row">
                <span class="integration-name"
                    ><strong>{client.label}</strong><small>{detail(client)}</small></span
                >
                <span class="integration-status" class:configured={connected(client)}
                    >{connected(client) ? 'Connected' : 'Off'}</span
                >
                {#if connected(client)}
                    <button
                        disabled={busy !== ''}
                        aria-label={`Disconnect ${client.label}`}
                        onclick={() => change(client, false)}
                        >{busy === client.id ? 'Removing…' : 'Disconnect'}</button
                    >
                {:else}
                    <button
                        class="primary"
                        disabled={busy !== '' || !client.installed}
                        aria-label={`Connect ${client.label}`}
                        onclick={() => change(client, true)}
                        >{busy === client.id ? 'Connecting…' : 'Connect'}</button
                    >
                {/if}
            </div>
        </li>
    {/each}
</ul>
{#if error}<p class="error" role="alert">{error}</p>{/if}
