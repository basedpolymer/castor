/*
 * Castor — small C shim over MuPDF for what the safe `mupdf` crate does not expose:
 * image rewriting / font subsetting / recolouring (compress, grayscale) and the built-in
 * Tesseract OCR. Every entry point runs inside fz_try and owns its fz_context, so no MuPDF
 * exception (longjmp) can ever cross a Rust frame.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
#include <string.h>
#include <stdio.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"

/* Internal MuPDF OCR entry points (source/fitz/tessocr.h); not in the public headers. */
void *ocr_init(fz_context *ctx, const char *lang, const char *datadir, const char *options);
void ocr_fin(fz_context *ctx, void *api);
void ocr_recognise(fz_context *ctx, void *api, fz_pixmap *pix,
	void (*callback)(fz_context *ctx, void *arg, int unicode, const char *font_name,
		const int *line_bbox, const int *word_bbox, const int *char_bbox, int pointsize),
	int (*progress)(fz_context *ctx, void *arg, int progress),
	void *arg);
/* Tesseract C API (compiled into MuPDF's thirdparty library). */
int *TessBaseAPIAllWordConfidences(void *handle);
void TessDeleteIntArray(const int *arr);
int TessBaseAPIMeanTextConf(void *handle);

static void quiet(void *user, const char *message) { (void)user; (void)message; }

static fz_context *new_ctx(void)
{
	fz_context *ctx = fz_new_context(NULL, NULL, FZ_STORE_DEFAULT);
	if (!ctx)
		return NULL;
	fz_set_warning_callback(ctx, quiet, NULL);
	fz_set_error_callback(ctx, quiet, NULL);
	fz_try(ctx)
		fz_register_document_handlers(ctx);
	fz_catch(ctx)
	{
		fz_drop_context(ctx);
		return NULL;
	}
	return ctx;
}

static void set_err(fz_context *ctx, char *err, int errlen)
{
	if (err && errlen > 0)
	{
		const char *m = ctx ? fz_caught_message(ctx) : "out of memory";
		snprintf(err, (size_t)errlen, "%s", m ? m : "unknown MuPDF error");
	}
}

typedef struct
{
	int recolor;          /* 0 = keep, 1 = gray, 3 = rgb (CMYK → RGB) */
	int subset_fonts;
	int rewrite_images;
	int color_threshold;  /* dpi above which colour/gray images are downsampled (0 = never) */
	int color_to;         /* target dpi */
	int lossless_to_jpeg; /* also recompress lossless colour/gray images as JPEG */
	int jpeg_quality;     /* 1..100 */
	int bitonal_threshold;
	int bitonal_to;
	int garbage;          /* 0..4 */
	int objstms;
	int effort;           /* 0..100 */
	int clean;
	int linear;
} castor_opt;

/* Open `in`, apply `o`, save to `out`. Returns 0 on success, -1 with a message in `err`. */
int castor_optimize(const char *in, const char *out, const char *password, const castor_opt *o, char *err, int errlen)
{
	fz_context *ctx = new_ctx();
	pdf_document *doc = NULL;
	char quality[16];
	int rc = 0;
	if (!ctx)
	{
		set_err(NULL, err, errlen);
		return -1;
	}
	snprintf(quality, sizeof quality, "%d", o->jpeg_quality > 0 ? o->jpeg_quality : 75);
	fz_var(doc);
	fz_try(ctx)
	{
		int i, n;
		pdf_write_options w = pdf_default_write_options;
		doc = pdf_open_document(ctx, in);
		if (pdf_needs_password(ctx, doc) && !pdf_authenticate_password(ctx, doc, password ? password : ""))
			fz_throw(ctx, FZ_ERROR_ARGUMENT, "password required");
		n = pdf_count_pages(ctx, doc);
		if (o->recolor == 1 || o->recolor == 3)
		{
			pdf_recolor_options ro;
			ro.num_comp = o->recolor;
			for (i = 0; i < n; i++)
				pdf_recolor_page(ctx, doc, i, &ro);
			if (o->recolor == 1)
				pdf_remove_output_intents(ctx, doc);
		}
		if (o->rewrite_images)
		{
			pdf_image_rewriter_options ro;
			memset(&ro, 0, sizeof ro);
			ro.color_lossy_image_subsample_method = FZ_SUBSAMPLE_BICUBIC;
			ro.color_lossless_image_subsample_method = FZ_SUBSAMPLE_BICUBIC;
			ro.gray_lossy_image_subsample_method = FZ_SUBSAMPLE_BICUBIC;
			ro.gray_lossless_image_subsample_method = FZ_SUBSAMPLE_BICUBIC;
			ro.bitonal_image_subsample_method = FZ_SUBSAMPLE_AVERAGE;
			ro.color_lossy_image_subsample_threshold = o->color_threshold;
			ro.color_lossy_image_subsample_to = o->color_to;
			ro.color_lossless_image_subsample_threshold = o->color_threshold;
			ro.color_lossless_image_subsample_to = o->color_to;
			ro.gray_lossy_image_subsample_threshold = o->color_threshold;
			ro.gray_lossy_image_subsample_to = o->color_to;
			ro.gray_lossless_image_subsample_threshold = o->color_threshold;
			ro.gray_lossless_image_subsample_to = o->color_to;
			ro.bitonal_image_subsample_threshold = o->bitonal_threshold;
			ro.bitonal_image_subsample_to = o->bitonal_to;
			ro.color_lossy_image_recompress_method = FZ_RECOMPRESS_JPEG;
			ro.gray_lossy_image_recompress_method = FZ_RECOMPRESS_JPEG;
			ro.color_lossless_image_recompress_method = o->lossless_to_jpeg ? FZ_RECOMPRESS_JPEG : FZ_RECOMPRESS_LOSSLESS;
			ro.gray_lossless_image_recompress_method = o->lossless_to_jpeg ? FZ_RECOMPRESS_JPEG : FZ_RECOMPRESS_LOSSLESS;
			ro.bitonal_image_recompress_method = FZ_RECOMPRESS_FAX;
			ro.color_lossy_image_recompress_quality = quality;
			ro.gray_lossy_image_recompress_quality = quality;
			ro.color_lossless_image_recompress_quality = quality;
			ro.gray_lossless_image_recompress_quality = quality;
			ro.recompress_when = FZ_RECOMPRESS_WHEN_SMALLER;
			pdf_rewrite_images(ctx, doc, &ro);
		}
		if (o->subset_fonts)
			pdf_subset_fonts(ctx, doc, 0, NULL);
		w.do_garbage = o->garbage;
		w.do_compress = 1;
		w.do_compress_images = 1;
		w.do_compress_fonts = 1;
		w.do_use_objstms = o->objstms;
		w.compression_effort = o->effort;
		w.do_clean = o->clean;
		w.do_linear = o->linear;
		pdf_save_document(ctx, doc, out, &w);
	}
	fz_always(ctx)
		pdf_drop_document(ctx, doc);
	fz_catch(ctx)
	{
		set_err(ctx, err, errlen);
		rc = -1;
	}
	fz_drop_context(ctx);
	return rc;
}

/* ---------------------------------------------------------------------------------------- */
/* OCR */

typedef struct
{
	fz_context *ctx;
	void *api;
} castor_ocr;

typedef void (castor_char_cb)(void *arg, int unicode, const int *line_bbox, const int *word_bbox, const int *char_bbox, int pointsize);
typedef int (castor_progress_cb)(void *arg, int percent);

castor_ocr *castor_ocr_new(const char *lang, const char *datadir, char *err, int errlen)
{
	castor_ocr *h;
	fz_context *ctx = new_ctx();
	if (!ctx)
	{
		set_err(NULL, err, errlen);
		return NULL;
	}
	h = calloc(1, sizeof *h);
	if (!h)
	{
		fz_drop_context(ctx);
		set_err(NULL, err, errlen);
		return NULL;
	}
	h->ctx = ctx;
	fz_try(ctx)
		h->api = ocr_init(ctx, lang, datadir, NULL);
	fz_catch(ctx)
	{
		set_err(ctx, err, errlen);
		fz_drop_context(ctx);
		free(h);
		return NULL;
	}
	return h;
}

void castor_ocr_free(castor_ocr *h)
{
	if (!h)
		return;
	fz_try(h->ctx)
		ocr_fin(h->ctx, h->api);
	fz_catch(h->ctx)
		; /* nothing sensible to do */
	fz_drop_context(h->ctx);
	free(h);
}

typedef struct
{
	castor_char_cb *cb;
	castor_progress_cb *progress;
	void *arg;
} ocr_call;

static void ocr_char(fz_context *ctx, void *arg, int unicode, const char *font_name,
	const int *line_bbox, const int *word_bbox, const int *char_bbox, int pointsize)
{
	ocr_call *c = (ocr_call *)arg;
	(void)ctx;
	(void)font_name;
	c->cb(c->arg, unicode, line_bbox, word_bbox, char_bbox, pointsize);
}

static int ocr_progress(fz_context *ctx, void *arg, int percent)
{
	ocr_call *c = (ocr_call *)arg;
	(void)ctx;
	return c->progress ? c->progress(c->arg, percent) : 0;
}

/* Recognise an 8-bit gray image (`stride` bytes per row). Characters are reported in reading
 * order with pixel boxes. `confs`/`nconfs` receive per-word confidences (0..100), to be freed
 * with castor_ocr_free_confs. Returns 0, or -1 with a message. */
int castor_ocr_run(castor_ocr *h, const unsigned char *gray, int w, int h_px, int stride, int dpi,
	castor_char_cb *cb, castor_progress_cb *progress, void *arg, int **confs, int *nconfs, int *mean_conf,
	char *err, int errlen)
{
	fz_context *ctx = h->ctx;
	fz_pixmap *pix = NULL;
	int rc = 0;
	ocr_call call;
	call.cb = cb;
	call.progress = progress;
	call.arg = arg;
	*confs = NULL;
	*nconfs = 0;
	*mean_conf = 0;
	fz_var(pix);
	fz_try(ctx)
	{
		int y, pw = (w + 3) & ~3;
		pix = fz_new_pixmap(ctx, fz_device_gray(ctx), pw, h_px, NULL, 0);
		for (y = 0; y < h_px; y++)
		{
			unsigned char *row = pix->samples + (size_t)y * pix->stride;
			memcpy(row, gray + (size_t)y * stride, (size_t)w);
			if (pw > w)
				memset(row + w, 255, (size_t)(pw - w));
		}
		pix->xres = pix->yres = dpi > 0 ? dpi : 300;
		ocr_recognise(ctx, h->api, pix, ocr_char, ocr_progress, &call);
		{
			int *c = TessBaseAPIAllWordConfidences(h->api);
			if (c)
			{
				int n = 0;
				while (c[n] != -1)
					n++;
				*confs = c;
				*nconfs = n;
			}
			*mean_conf = TessBaseAPIMeanTextConf(h->api);
		}
	}
	fz_always(ctx)
		fz_drop_pixmap(ctx, pix);
	fz_catch(ctx)
	{
		set_err(ctx, err, errlen);
		rc = -1;
	}
	return rc;
}

void castor_ocr_free_confs(int *confs)
{
	if (confs)
		TessDeleteIntArray(confs);
}
