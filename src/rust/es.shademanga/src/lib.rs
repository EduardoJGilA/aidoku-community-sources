#![no_std]

extern crate alloc;
use alloc::{format, string::ToString};

use aidoku::{
	error::{AidokuError, AidokuErrorKind, Result},
	prelude::*,
	std::{
		html::unescape_html_entities,
		net::{HttpMethod, Request},
		ObjectRef, String, ValueRef, Vec,
	},
	Chapter, Filter, FilterType, Listing, Manga, MangaContentRating, MangaPageResult, MangaStatus,
	MangaViewer, Page,
};

static USER_AGENT: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
static BASE_URL: &str = "https://shademanga.com";
static API_URL: &str = "https://shademanga.com/api";
static CDN_REFERER: &str = "https://shademanga.com";

const PAGE_LIMIT: i32 = 30;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn api_get(path: &str) -> Result<ValueRef> {
	let url = format!("{API_URL}{path}");
	Request::new(url.as_str(), HttpMethod::Get)
		.header("User-Agent", USER_AGENT)
		.header("Accept", "application/json")
		.header("Referer", BASE_URL)
		.header("Origin", BASE_URL)
		.json()
}

fn build_query(parts: &[(&str, String)]) -> String {
	if parts.is_empty() {
		return String::new();
	}
	let mut out = String::from("?");
	for (i, (k, v)) in parts.iter().enumerate() {
		if i > 0 {
			out.push('&');
		}
		out.push_str(k);
		out.push('=');
		out.push_str(&url_encode(&v));
	}
	out
}

fn url_encode(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(b as char);
			}
			b' ' => out.push('+'),
			_ => {
				out.push('%');
				const HEX: &[u8; 16] = b"0123456789ABCDEF";
				out.push(HEX[(b >> 4) as usize] as char);
				out.push(HEX[(b & 0x0F) as usize] as char);
			}
		}
	}
	out
}

fn split_author(raw: &str) -> (String, String) {
	let cleaned = raw.replace('&', ",");
	let mut parts = cleaned
		.split(',')
		.map(|s| s.trim())
		.filter(|s| !s.is_empty())
		.map(|s| s.to_string())
		.collect::<Vec<_>>();
	if parts.is_empty() {
		return (String::new(), String::new());
	}
	let author = parts.remove(0);
	let artist = parts.join(", ");
	(author, artist)
}

fn split_genres(raw: &str) -> Vec<String> {
	raw.split(',')
		.map(|s| s.trim().trim_end_matches('.').to_string())
		.filter(|s| !s.is_empty())
		.collect()
}

fn status_from_text(text: &str) -> MangaStatus {
	match text {
		"En curso" | "En emisión" | "Publicándose" | "Publishing" => MangaStatus::Ongoing,
		"Finalizado" | "Completado" | "Ended" | "Completed" => MangaStatus::Completed,
		"Pausado" | "En pausa" | "Hiatus" => MangaStatus::Hiatus,
		"Cancelado" | "Abandonado" | "Cancelled" => MangaStatus::Cancelled,
		_ => MangaStatus::Unknown,
	}
}

fn viewer_from_type(text: &str) -> MangaViewer {
	let lc = text.to_ascii_lowercase();
	match lc.as_str() {
		"manhwa" | "manhua" | "webtoon" => MangaViewer::Scroll,
		_ => MangaViewer::Rtl,
	}
}

fn unescape(s: String) -> String {
	unescape_html_entities(s)
}

// ---------------------------------------------------------------------------
// Listing / search / detail / chapter / pages
// ---------------------------------------------------------------------------

fn parse_manga_item(item: &ObjectRef) -> Option<Manga> {
	let id = item.get("id").as_int().ok()?;
	let id_str = id.to_string();
	let title = item
		.get("titulo")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	if title.is_empty() {
		return None;
	}
	let cover = item
		.get("portadaUrl")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let autor_raw = item
		.get("autor")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let (author, artist) = split_author(&autor_raw);

	let url = format!("{BASE_URL}/serie/local/{id}");

	let nsfw = item
		.get("esMayorDeEdad")
		.as_bool()
		.unwrap_or(false);

	let status = if item.get("esNuevo").as_bool().unwrap_or(false) {
		MangaStatus::Ongoing
	} else {
		MangaStatus::Unknown
	};

	Some(Manga {
		id: id_str,
		cover,
		title,
		author,
		artist,
		description: String::new(),
		url,
		categories: Vec::new(),
		status,
		nsfw: if nsfw {
			MangaContentRating::Nsfw
		} else {
			MangaContentRating::Safe
		},
		viewer: MangaViewer::Rtl,
	})
}

fn parse_grouped_array(value: ValueRef) -> Vec<Manga> {
	let mut out: Vec<Manga> = Vec::new();
	if let Ok(arr) = value.as_array() {
		for group in arr {
			let group = match group.as_object() {
				Ok(g) => g,
				Err(_) => continue,
			};
			let series = match group.get("series").as_array() {
				Ok(s) => s,
				Err(_) => continue,
			};
			for item in series {
				if let Ok(obj) = item.as_object() {
					if let Some(m) = parse_manga_item(&obj) {
						out.push(m);
					}
				}
			}
		}
	}
	out
}

fn parse_flat_array(value: ValueRef) -> Vec<Manga> {
	let mut out: Vec<Manga> = Vec::new();
	if let Ok(arr) = value.as_array() {
		for item in arr {
			if let Ok(obj) = item.as_object() {
				if let Some(m) = parse_manga_item(&obj) {
					out.push(m);
				}
			}
		}
	}
	out
}

#[get_manga_list]
fn get_manga_list(filters: Vec<Filter>, page: i32) -> Result<MangaPageResult> {
	let mut query: Option<String> = None;
	let mut genero: Option<String> = None;
	let mut tipo: Option<String> = None;
	let mut orden: Option<String> = None;
	let mut include_adult: bool = false;

	for filter in filters {
		match filter.kind {
			FilterType::Title => {
				if let Ok(v) = filter.value.as_string() {
					let s = v.read();
					if !s.is_empty() {
						query = Some(s);
					}
				}
			}
			FilterType::Genre => {
				if let Ok(id) = filter.object.get("id").as_string() {
					let s = id.read();
					if !s.is_empty() {
						genero = Some(s);
					}
				}
			}
			FilterType::Check => {
				if filter.name.as_str() == "Contenido adulto" {
					if let Ok(v) = filter.value.as_int() {
						include_adult = v != 0;
					}
				}
			}
			FilterType::Select => {
				let value = match filter.value.as_int() {
					Ok(v) => v,
					Err(_) => continue,
				};
				match filter.name.as_str() {
					"Tipo" => {
						tipo = match value {
							1 => Some(String::from("Manga")),
							2 => Some(String::from("Manhua")),
							3 => Some(String::from("Manhwa")),
							4 => Some(String::from("Novela")),
							5 => Some(String::from("One shot")),
							6 => Some(String::from("Doujinshi")),
							7 => Some(String::from("Oel")),
							_ => None,
						};
					}
					_ => continue,
				}
			}
			FilterType::Sort => {
				let value = match filter.value.as_object() {
					Ok(v) => v,
					Err(_) => continue,
				};
				let index = value.get("index").as_int().unwrap_or(0);
				orden = match index {
					0 => None, // default server order (popularity)
					1 => Some(String::from("fechaActualizacion")),
					2 => Some(String::from("titulo")),
					3 => Some(String::from("puntuacion")),
					_ => None,
				};
			}
			_ => continue,
		}
	}

	let page_num = if page < 1 { 1 } else { page };

	let mut parts: Vec<(&str, String)> = Vec::new();
	parts.push(("page", page_num.to_string()));
	parts.push(("limit", PAGE_LIMIT.to_string()));
	if let Some(q) = query {
		parts.push(("q", q));
	}
	if let Some(g) = genero {
		parts.push(("genero", g));
	}
	if let Some(t) = tipo {
		parts.push(("tipo", t));
	}
	if let Some(o) = orden {
		parts.push(("orden", o));
	}
	if include_adult {
		parts.push(("includeAdult", String::from("true")));
	}

	let qs = build_query(&parts);
	let json = api_get(&format!("/series-locales{qs}"))?;
	let manga = parse_flat_array(json);
	let has_more = manga.len() >= PAGE_LIMIT as usize;

	Ok(MangaPageResult { manga, has_more })
}

#[get_manga_listing]
fn get_manga_listing(listing: Listing, page: i32) -> Result<MangaPageResult> {
	let page_num = if page < 1 { 1 } else { page };
	let path = match listing.name.as_str() {
		"Populares" => format!("/series-locales/popular?page={page_num}&limit={PAGE_LIMIT}"),
		"Novedades" => format!("/series-locales/novedades?page={page_num}&limit={PAGE_LIMIT}"),
		_ => format!("/series-locales?page={page_num}&limit={PAGE_LIMIT}"),
	};
	let json = api_get(&path)?;
	let manga = parse_grouped_array(json);
	let has_more = manga.len() >= PAGE_LIMIT as usize;
	Ok(MangaPageResult { manga, has_more })
}

#[get_manga_details]
fn get_manga_details(id: String) -> Result<Manga> {
	let path = format!("/series-locales/{id}");
	let json = api_get(&path)?;
	let obj = json.as_object()?;

	let title = obj
		.get("titulo")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	if title.is_empty() {
		return Err(AidokuError {
			reason: AidokuErrorKind::JsonParseError,
		});
	}

	let cover = obj
		.get("portadaUrl")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let autor_raw = obj
		.get("autor")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let (author, artist) = split_author(&autor_raw);
	let description = unescape(
		obj.get("descripcion")
			.as_string()
			.map(|s| s.read())
			.unwrap_or_default(),
	);
	let generos_raw = obj
		.get("generos")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let categories = split_genres(&generos_raw);
	let estado = obj
		.get("estado")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let en_emision = obj.get("estaEnEmision").as_bool().unwrap_or(false);
	let status = if en_emision {
		MangaStatus::Ongoing
	} else {
		status_from_text(&estado)
	};
	let tipo = obj
		.get("tipo")
		.as_string()
		.map(|s| s.read())
		.unwrap_or_default();
	let viewer = viewer_from_type(&tipo);
	let es_mayor = obj.get("esMayorDeEdad").as_bool().unwrap_or(false);
	let nsfw = if es_mayor {
		MangaContentRating::Nsfw
	} else {
		MangaContentRating::Safe
	};

	let url = format!("{BASE_URL}/serie/local/{id}");

	Ok(Manga {
		id,
		cover,
		title,
		author,
		artist,
		description,
		url,
		categories,
		status,
		nsfw,
		viewer,
	})
}

#[get_chapter_list]
fn get_chapter_list(id: String) -> Result<Vec<Chapter>> {
	let path = format!("/series-locales/{id}/capitulos");
	let json = api_get(&path)?;
	let arr = json.as_array()?;

	let mut chapters: Vec<Chapter> = Vec::new();
	for item in arr {
		let obj = match item.as_object() {
			Ok(o) => o,
			Err(_) => continue,
		};
		let cap_id = match obj.get("id").as_int() {
			Ok(v) => v,
			Err(_) => continue,
		};
		let numero = obj.get("numeroCapitulo").as_float().unwrap_or(0.0) as f32;
		let titulo = obj
			.get("titulo")
			.as_string()
			.map(|s| s.read())
			.unwrap_or_default();
		let scanlator = obj
			.get("grupoScan")
			.as_object()
			.ok()
			.and_then(|g| g.get("nombre").as_string().ok())
			.map(|s| s.read())
			.unwrap_or_default();

		let ch_id = format!("{id}:{cap_id}");
		let ch_url = format!("{BASE_URL}/serie/local/{id}/capitulo/{cap_id}");

		chapters.push(Chapter {
			id: ch_id,
			title: titulo,
			chapter: numero,
			date_updated: obj
				.get("fechaSubida")
				.as_date("yyyy-MM-dd'T'HH:mm:ss", None, None)
				.unwrap_or(-1.0),
			scanlator,
			url: ch_url,
			lang: String::from("es"),
			..Default::default()
		});
	}

	Ok(chapters)
}

#[get_page_list]
fn get_page_list(manga_id: String, chapter_id: String) -> Result<Vec<Page>> {
	// chapter_id is "manga_id:chapter_id"
	let (mid, cid) = if let Some((m, c)) = chapter_id.split_once(':') {
		(m.to_string(), c.to_string())
	} else {
		(manga_id, chapter_id)
	};
	let path = format!("/series-locales/{mid}/capitulos/{cid}/paginas");
	let json = api_get(&path)?;
	let obj = json.as_object()?;
	let arr = obj.get("paginas").as_array()?;

	let mut pages: Vec<Page> = Vec::new();
	for (i, item) in arr.enumerate() {
		let url = match item.as_string() {
			Ok(s) => s.read(),
			Err(_) => continue,
		};
		pages.push(Page {
			index: i as i32,
			url,
			..Default::default()
		});
	}
	Ok(pages)
}

#[modify_image_request]
fn modify_image_request(request: Request) {
	request
		.header("User-Agent", USER_AGENT)
		.header("Referer", CDN_REFERER)
		.header("Origin", CDN_REFERER);
}
