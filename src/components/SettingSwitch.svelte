<script lang="ts">
    let {
        title,
        description,
        checked = $bindable(false),
        disabled = false,
    }: {
        title: string;
        description: string;
        checked: boolean;
        disabled?: boolean;
    } = $props();
    const id = $props.id();
</script>

<label class="setting-switch" class:disabled>
    <span class="switch-copy">
        <strong>{title}</strong>
        <span id={`${id}-description`}>{description}</span>
    </span>
    <input
        type="checkbox"
        aria-label={title}
        aria-describedby={`${id}-description`}
        bind:checked
        {disabled}
    />
</label>

<style>
    .setting-switch {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 24px;
        padding: 12px 0;
        cursor: pointer;
    }
    :global(.setting-switch) + .setting-switch {
        border-top: 1px solid var(--border);
    }
    .switch-copy {
        display: grid;
        gap: 5px;
    }
    strong {
        font-size: 13px;
        font-weight: 550;
    }
    .switch-copy > span {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.4;
    }
    /* Reset the global text-input sizing so the switch keeps its own shape. */
    input {
        appearance: none;
        -webkit-appearance: none;
        position: relative;
        width: 36px;
        height: 22px;
        min-height: 0;
        flex: 0 0 36px;
        padding: 0;
        margin: 0;
        border: 0;
        border-radius: 11px;
        background: var(--track);
        box-shadow: inset 0 0 0 1px #0000000d;
        cursor: inherit;
        transition: background-color 160ms ease;
    }
    input::before {
        content: '';
        position: absolute;
        top: 2px;
        left: 2px;
        width: 18px;
        height: 18px;
        background: #fff;
        border-radius: 50%;
        box-shadow:
            0 1px 2px #0000002e,
            0 0 0 0.5px #0000000a;
        transition: transform 160ms ease;
    }
    input:checked {
        background: var(--accent);
        box-shadow: none;
    }
    input:checked::before {
        transform: translateX(14px);
    }
    input:focus {
        border: 0;
    }
    input:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: 3px;
    }
    .disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }
</style>
