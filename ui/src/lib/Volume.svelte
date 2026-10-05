<script lang="ts">
  import Card from './Card.svelte';
  import Slider from './Slider.svelte';
  import Toggle from './Toggle.svelte';
  import { t } from './i18n';
  import { store } from './store.svelte';

  let s = $derived(store.state!);
  let micName = $derived([...s.devices, ...s.hidden_devices].find((d) => d.id === s.active_mic)?.name ?? s.active_mic ?? '');
  let outName = $derived(s.sinks.find((d) => d.id === s.output_device)?.name ?? s.output_device ?? '');

  // Sliders follow the device, but while dragging they show the dragged value and only send the
  // last one; each device remembers its own level.
  let drag = $state<Record<string, number>>({});
  const timers: Record<string, ReturnType<typeof setTimeout>> = {};
  function set(device: string, v: number) {
    drag[device] = v;
    clearTimeout(timers[device]);
    timers[device] = setTimeout(() => {
      store.apply({ device_levels: { [device]: Math.round(v * 100) / 100 } });
      setTimeout(() => delete drag[device], 600);
    }, 150);
  }
  const pct = (v: number) => `${Math.round(v * 100)} %`;
</script>

{#if (s.mic_volume !== null && s.active_mic) || (s.output_volume !== null && s.output_device)}
  <Card title={t('volume.title')} sub={t('volume.sub')}>
    <div class="body">
      {#if s.mic_volume !== null && s.active_mic}
        <Slider
          label={`${t('volume.mic')} · ${micName}`}
          min={0}
          max={1}
          step={0.01}
          value={drag[s.active_mic] ?? s.mic_volume}
          format={pct}
          onchange={(v) => set(s.active_mic!, v)}
        />
        <div class="line">
          <div>
            <span>{t('level.guard')}</span>
            <small>{t('level.guard_sub')}</small>
          </div>
          <Toggle
            label={t('level.guard')}
            checked={s.settings.mic_level.avoid_clipping}
            onchange={(v) => store.apply({ mic_level: { avoid_clipping: v } })}
          />
        </div>
      {/if}
      {#if s.output_volume !== null && s.output_device}
        <Slider
          label={`${t('volume.out')} · ${outName}`}
          min={0}
          max={1}
          step={0.01}
          value={drag[s.output_device] ?? s.output_volume}
          format={pct}
          onchange={(v) => set(s.output_device!, v)}
        />
      {/if}
    </div>
  </Card>
{/if}

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .line {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    font-size: 13.5px;
  }
  small {
    display: block;
    color: var(--sub);
    font-size: 12.5px;
  }
</style>
