<script lang="ts">
    let {
        name,
        size = 18,
    }: { name: 'bug' | 'activity' | 'settings' | 'app' | 'code' | 'model'; size?: number } =
        $props();
    const paths = {
        bug: 'M8 7h8M9 3l1 3m5-3-1 3M8 10H4m12 0h4M8 14H4m12 0h4M8 18l-3 2m11-2 3 2M12 8v12M7 12c0-4 2-6 5-6s5 2 5 6v3c0 4-2 6-5 6s-5-2-5-6z',
        activity: 'M3 12h4l3-8 4 16 3-8h4',
        settings:
            'M19.3,12.0 19.2,12.9 21.3,13.8 20.5,16.2 18.3,15.6 17.2,17.2 16.5,17.8 17.3,19.9 15.1,21.0 13.9,19.1 12.0,19.3 11.1,19.2 10.2,21.3 7.8,20.5 8.4,18.3 6.8,17.2 6.2,16.5 4.1,17.3 3.0,15.1 4.9,13.9 4.7,12.0 4.8,11.1 2.7,10.2 3.5,7.8 5.7,8.3 6.8,6.8 7.5,6.2 6.7,4.1 8.9,3.0 10.1,4.9 12.0,4.7 12.9,4.8 13.8,2.7 16.2,3.5 15.7,5.7 17.2,6.8 17.8,7.5 19.9,6.7 21.0,8.9 19.1,10.1zM12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7',
        app: 'M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1M4 9h16M8 6.5h.01',
        code: 'm8 6-6 6 6 6m8-12 6 6-6 6M14 3l-4 18',
        model: 'm12 3 9 9-9 9-9-9zM8 12h8m-4-4v8',
    };
</script>

<svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"><path d={paths[name]} /></svg
>
