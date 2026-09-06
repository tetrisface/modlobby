import { For, Show, createResource, createSignal, onMount } from 'solid-js'
import type { NewsItem } from '../ipc/bindings/NewsItem'
import { openExternal } from '../components/Linkify'
import { loadNews, markNewsRead, news, newsBanner } from '../store/news'

/**
 * BAR's news, as the website publishes it.
 *
 * The stories live on the site and are read there — a lobby is not a browser,
 * and the feed carries a summary rather than the article — so a card is a way
 * in rather than the thing itself.
 *
 * Opening the page is what marks it read. There is no per-story read state:
 * this is a page you glance at, and a count that only goes down as far as you
 * scrolled would be a chore rather than a hint.
 */
export function News() {
  const [items] = createResource(async () => {
    await loadNews()
    return news()?.items ?? []
  })
  onMount(() => void markNewsRead())

  return (
    <section class='news'>
      <h1>News</h1>
      <Show
        when={(items() ?? []).length > 0}
        fallback={
          <p class='muted'>
            <Show when={!items.loading} fallback='Fetching the news…'>
              No news right now. It is fetched from beyondallreason.info, so
              this is what an offline launch looks like too.
            </Show>
          </p>
        }
      >
        <div class='news-list'>
          <For each={items()}>{(item) => <Story item={item} />}</For>
        </div>
      </Show>
    </section>
  )
}

function Story(props: { item: NewsItem }) {
  // A picture the CDN would not give us is no reason to show a broken card:
  // the story reads perfectly well without one, and the layout closes up.
  const [broken, setBroken] = createSignal(false)
  const banner = () =>
    broken() || !props.item.image ? null : newsBanner(props.item.id)
  /** The day it went up, in the reader's own format. */
  const when = () => {
    const at = props.item.publishedAt
    return at === null ? '' : new Date(at * 1000).toLocaleDateString()
  }

  return (
    <article class='news-card'>
      <Show when={banner()}>
        {(src) => (
          <img
            class='news-banner'
            src={src()}
            alt=''
            loading='lazy'
            decoding='async'
            onError={() => setBroken(true)}
          />
        )}
      </Show>
      <div class='news-text'>
        <h2>
          <a
            href={props.item.link}
            title={props.item.link}
            onClick={(event) => {
              event.preventDefault()
              void openExternal(props.item.link)
            }}
          >
            {props.item.title}
          </a>
        </h2>
        <Show when={when()}>
          <p class='news-when'>{when()}</p>
        </Show>
        <Show when={props.item.summary}>
          <p class='news-summary'>{props.item.summary}</p>
        </Show>
      </div>
    </article>
  )
}
