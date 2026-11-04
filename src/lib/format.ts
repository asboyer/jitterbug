export function memory(bytes: number): string {
    // 1000 MB and up reads as GB, so a value never prints as a four-digit MB figure.
    if (bytes >= 1000 * 1024 ** 2) {
        return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    }
    return `${Math.round(bytes / 1024 ** 2)} MB`;
}

export function percent(value: number): string {
    return value > 0 && value < 0.1 ? '<0.1%' : `${value.toFixed(1)}%`;
}

export function errorMessage(error: unknown): string {
    return typeof error === 'string' ? error : 'Something went wrong. Please try again.';
}

/** "3 min ago" style relative time for a Unix timestamp in seconds. */
export function ago(seconds: number, now = Date.now() / 1000): string {
    const delta = Math.max(0, Math.round(now - seconds));
    if (delta < 45) return 'just now';
    if (delta < 3600) return `${Math.round(delta / 60)} min ago`;
    if (delta < 86400) return `${Math.round(delta / 3600)} h ago`;
    return `${Math.round(delta / 86400)} d ago`;
}
