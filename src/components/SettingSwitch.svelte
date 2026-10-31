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
    input {
        appearance: none;
        -webkit-appearance: none;
        width: 32px;
        height: 20px;
        flex: 0 0 32px;
        padding: 2px;
        margin: 0;
        border: 0;
        border-radius: 20px;
        background: var(--track);
        cursor: inherit;
        transition: background 120ms;
    }
    input::before {
        content: '';
        display: block;
        width: 16px;
        height: 16px;
        background: #fff;
        border-radius: 50%;
        box-shadow: 0 1px 3px #0003;
        transition: transform 120ms;
    }
    input:checked {
        background: var(--accent);
    }
    input:checked::before {
        transform: translateX(12px);
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
