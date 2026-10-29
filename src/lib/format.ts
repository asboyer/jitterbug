export function memory(bytes: number): string {
    if (bytes >= 1024 ** 3) {
        return `${(bytes / 1024 ** 3).toFixed(1)} GiB`;
    }
    return `${Math.round(bytes / 1024 ** 2)} MiB`;
}

export function percent(value: number): string {
    return `${value.toFixed(1)}%`;
}

export function errorMessage(error: unknown): string {
    return typeof error === 'string' ? error : 'Something went wrong. Please try again.';
}
