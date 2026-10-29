<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { tourSteps } from '../lib/tour';

    let {
        index,
        onnext,
        onback,
        onclose,
    }: {
        index: number;
        onnext: () => void;
        onback: () => void;
        onclose: () => void;
    } = $props();
    let dialog: HTMLDialogElement;
    let card: HTMLDivElement;
    let next: HTMLButtonElement;
    let highlight = $state<{ left: number; top: number; width: number; height: number } | null>(
        null
    );
    let position = $state({ left: 24, top: 80 });
    const step = $derived(tourSteps[index]);

    onMount(() => {
        dialog.showModal();
        next.focus();
        return () => dialog.close();
    });

    $effect(() => {
        const target = step.target;
        let disposed = false;
        let observer: ResizeObserver | undefined;
        const clamp = (value: number, min: number, max: number) =>
            Math.max(min, Math.min(value, max));
        function measure() {
            if (disposed) return;
            const element = document.querySelector<HTMLElement>(`[data-tour="${target}"]`);
            const box = element?.getBoundingClientRect();
            const width = card?.getBoundingClientRect().width ?? 350;
            const height = card?.getBoundingClientRect().height ?? 260;
            if (!box) {
                highlight = null;
                position = { left: (innerWidth - width) / 2, top: (innerHeight - height) / 2 };
                return;
            }
            const left = clamp(box.left - 6, 8, innerWidth - 16);
            const top = clamp(box.top - 6, 8, innerHeight - 16);
            highlight = {
                left,
                top,
                width: Math.max(8, Math.min(box.right + 6, innerWidth - 8) - left),
                height: Math.max(8, Math.min(box.bottom + 6, innerHeight - 8) - top),
            };
            const side = box.left < 200 && box.width < 230;
            let y = side ? box.top : box.bottom + 18;
            if (!side && y + height > innerHeight - 18) y = box.top - height - 18;
            position = {
                left: clamp(side ? box.right + 18 : box.right - width, 18, innerWidth - width - 18),
                top: clamp(y, 18, innerHeight - height - 18),
            };
        }
        void tick().then(() => {
            if (disposed) return;
            const element = document.querySelector<HTMLElement>(`[data-tour="${target}"]`);
            element?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
            measure();
            observer = new ResizeObserver(measure);
            if (element) observer.observe(element);
            if (card) observer.observe(card);
            next?.focus();
        });
        window.addEventListener('resize', measure);
        document.addEventListener('scroll', measure, true);
        return () => {
            disposed = true;
            observer?.disconnect();
            window.removeEventListener('resize', measure);
            document.removeEventListener('scroll', measure, true);
        };
    });
</script>

<dialog
    bind:this={dialog}
    class="tour-layer"
    aria-label="Jitterbug tour"
    aria-describedby="tour-body"
    oncancel={(event) => {
        event.preventDefault();
        onclose();
    }}
>
    {#if highlight}<div
            class="spotlight"
            aria-hidden="true"
            style={`left:${highlight.left}px;top:${highlight.top}px;width:${highlight.width}px;height:${highlight.height}px`}
        ></div>{:else}<div class="tour-shade"></div>{/if}
    <div class="tour-card" bind:this={card} style={`left:${position.left}px;top:${position.top}px`}>
        <div class="tour-meta">
            <span>GUIDED TOUR · EXAMPLE DATA</span><button
                class="icon-button"
                aria-label="Close tour"
                onclick={onclose}>×</button
            >
        </div>
        <div aria-live="polite">
            <h2>{step.title}</h2>
            <p id="tour-body">{step.body}</p>
        </div>
        <div class="tour-progress" aria-label={`Step ${index + 1} of ${tourSteps.length}`}>
            {#each tourSteps as _, i}<span class:current={i === index}></span>{/each}
            <small>{index + 1} / {tourSteps.length}</small>
        </div>
        <div class="tour-actions">
            <button class="text-button skip" onclick={onclose}>Skip tour</button><button
                disabled={index === 0}
                onclick={onback}>Back</button
            ><button bind:this={next} class="primary" onclick={onnext}
                >{index === tourSteps.length - 1 ? 'Finish' : 'Next'}</button
            >
        </div>
    </div>
</dialog>

<style>
    .tour-layer {
        position: fixed;
        inset: 0;
        width: 100vw;
        height: 100vh;
        max-width: none;
        max-height: none;
        padding: 0;
        margin: 0;
        border: 0;
        background: transparent;
        color: var(--text);
        overflow: hidden;
    }
    .tour-layer::backdrop {
        background: transparent;
    }
    .spotlight {
        position: fixed;
        border: 2px solid var(--accent);
        border-radius: 11px;
        box-shadow: 0 0 0 9999px #0009;
        pointer-events: none;
    }
    .tour-shade {
        position: fixed;
        inset: 0;
        background: #0009;
    }
    .tour-card {
        position: fixed;
        width: min(350px, calc(100vw - 36px));
        max-height: calc(100vh - 36px);
        overflow: auto;
        background: var(--panel);
        border: 1px solid var(--border);
        padding: 20px;
        border-radius: 14px;
        box-shadow: 0 20px 70px #0005;
    }
    .tour-meta {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        color: var(--muted);
        font-size: 9px;
        letter-spacing: 0.5px;
    }
    h2 {
        font-size: 19px;
        line-height: 1.3;
        margin: 14px 0 10px;
    }
    p {
        font-size: 13px;
        line-height: 1.6;
        margin: 0;
        color: var(--muted);
    }
    .tour-progress {
        display: flex;
        align-items: center;
        gap: 5px;
        margin-top: 20px;
    }
    .tour-progress > span {
        width: 6px;
        height: 6px;
        background: var(--track);
        border-radius: 50%;
    }
    .tour-progress > span.current {
        background: var(--accent);
    }
    .tour-progress small {
        color: var(--muted);
        font-size: 10px;
        margin-left: auto;
    }
    .tour-actions {
        display: flex;
        gap: 8px;
        margin-top: 18px;
    }
    .tour-actions .skip {
        margin-right: auto;
    }
</style>
