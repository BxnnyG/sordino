<script lang="ts">
  import Card from './Card.svelte';
  import Slider from './Slider.svelte';
  import Toggle from './Toggle.svelte';
  import { api } from './api';
  import { store } from './store.svelte';
  import { t } from './i18n';
  import type { Strength } from './types';

  let s = $derived(store.state!);
  let sp = $derived(s.settings.speaker);
  const strengths: Strength[] = ['light', 'medium', 'high', 'max'];
  let outputs = $derived(s.sinks.filter((d) => d.id !== 'sordino_speaker'));
  let playingTo = $derived(outputs.find((d) => d.id === s.speaker_output)?.name ?? s.speaker_output);
  let isDefault = $derived(s.default_sink === 'sordino_speaker');
</script>

<Card title={t('speaker.title')} sub={t('speaker.sub')}>
  {#snippet control()}
    <Toggle label={t('speaker.title')} checked={sp.enabled} onchange={(v) => store.apply({ speaker: { enabled: v } })} />
  {/snippet}
  {#if sp.enabled}
    <div class="body">
      <Slider
        label={t('noise.strength')}
        min={0}
        max={3}
        step={1}
        value={strengths.indexOf(sp.strength)}
        format={(v) => t(`strength.${strengths[v]}`)}
        onchange={(v) => store.apply({ speaker: { strength: strengths[v] } })}
        ticks={strengths.map((x) => t(`strength.${x}`))}
      />
      <label class="pick">
        <span>{t('speaker.plays_to')}</span>
        <select value={sp.output ?? ''} onchange={(e) => store.apply({ speaker: { output: e.currentTarget.value || null } })}>
          <option value="">{t('speaker.auto')}{sp.output === null && playingTo ? ` (${playingTo})` : ''}</option>
          {#each outputs as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </label>
      {#if s.speaker_active && !s.speaker_output}<p class="warn">{t('speaker.no_output')}</p>{/if}
      {#if isDefault}
        <p class="ok">{t('speaker.is_default')}</p>
      {:else}
        <p class="hint">{t('speaker.how')}</p>
        <button class="secondary" onclick={() => api.setDefaultDevice('sink', 'sordino_speaker')}>{t('speaker.make_default')}</button>
      {/if}
    </div>
  {/if}
</Card>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .pick {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13.5px;
  }
  .pick select {
    appearance: none;
    -webkit-appearance: none;
    border: 0;
    border-radius: 12px;
    background: var(--track) url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='12' height='8' viewBox='0 0 12 8'><path d='M1 1l5 5 5-5' fill='none' stroke='%23888' stroke-width='1.8' stroke-linecap='round'/></svg>") no-repeat right 14px center;
    color: var(--text);
    padding: 11px 38px 11px 14px;
    font: inherit;
    cursor: pointer;
  }
  .hint,
  .ok,
  .warn {
    margin: 0;
    font-size: 12.5px;
    color: var(--sub);
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .secondary {
    align-self: flex-start;
    border: 0;
    border-radius: 10px;
    padding: 8px 14px;
    background: var(--track);
    color: var(--text);
    font-weight: 500;
    cursor: pointer;
  }
</style>
