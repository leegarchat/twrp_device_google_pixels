/* fox_fbe_kdf — KDF playground for FBE synthetic-password research.
 *
 * Usage: fox_fbe_kdf <secret-hex> "<pipeline>"
 * Prints the resulting key as lowercase hex + newline on stdout.
 *
 * Pipeline stages separated by '|', left to right. Input starts as the
 * decoded secret bytes. Supported stages (space-separated args):
 *   slice A B        bytes [A:B)
 *   hex              bytes -> lowercase hex text
 *   unhex            hex text -> bytes
 *   ph512 [label]    PersonalizedHash-SHA512 (128B zero pad + label + key)
 *   ph256 [label]    same with SHA256
 *   sp800 L CTX PRF LBITS
 *                    SP800-108 counter KDF: HMAC(PRF) over
 *                    counter||label||0x00||ctx||ctxbits||Lbits
 *   sha256 | sha512 raw digest bytes
 *   xorhalf          XOR the two halves (even length required)
 * Unknown stage -> exit 2. All errors -> exit 1, no output.
 *
 * Lets recovery try new KDF ideas by pushing a text config, without
 * rebuilding the recovery image.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <openssl/hmac.h>
#include <openssl/evp.h>
#include <openssl/sha.h>

#define MAXBUF 256
#define MAXSTAGES 16
#define MAXARGS 8

static unsigned char g_buf[MAXBUF];
static size_t g_len = 0;

static int from_hex(const char* s, unsigned char* out, size_t cap) {
    size_t n = strlen(s);
    size_t i;
    if (n == 0 || (n & 1) || n / 2 > cap) return -1;
    for (i = 0; i < n; i += 2) {
        unsigned v;
        if (sscanf(s + i, "%2x", &v) != 1) return -1;
        out[i / 2] = (unsigned char)v;
    }
    return (int)(n / 2);
}

static int cmp_label(const char* a, const char* b) {
    return strcmp(a, b) == 0;
}

/* PersonalizedHash mirror (HashPassword.cpp): SHA512/SHA256 over
 * 128-byte zeroed buffer with label at [0..] and key at [+128]. */
static int do_ph(const char* label, int sha256, unsigned char* out, size_t* outlen) {
    unsigned char pad[128];
    size_t llen;
    memset(pad, 0, sizeof(pad));
    llen = strlen(label);
    if (llen > sizeof(pad)) return -1;
    memcpy(pad, label, llen);
    if (sha256) {
        SHA256_CTX ctx;
        unsigned char h[SHA256_DIGEST_LENGTH];
        SHA256_Init(&ctx);
        SHA256_Update(&ctx, pad, sizeof(pad));
        SHA256_Update(&ctx, g_buf, g_len);
        SHA256_Final(h, &ctx);
        if (sizeof(h) > MAXBUF) return -1;
        memcpy(out, h, sizeof(h));
        *outlen = sizeof(h);
    } else {
        SHA512_CTX ctx;
        unsigned char h[SHA512_DIGEST_LENGTH];
        SHA512_Init(&ctx);
        SHA512_Update(&ctx, pad, sizeof(pad));
        SHA512_Update(&ctx, g_buf, g_len);
        SHA512_Final(h, &ctx);
        if (sizeof(h) > MAXBUF) return -1;
        memcpy(out, h, sizeof(h));
        *outlen = sizeof(h);
    }
    return 0;
}

/* SP800-108 counter mode. prf: 0 = HMAC-SHA256, 1 = HMAC-SHA512. */
static int do_sp800(const char* label, const char* ctx, int prf, unsigned lbits,
                    unsigned char* out, size_t* outlen) {
    const EVP_MD* md = prf ? EVP_sha512() : EVP_sha256();
    size_t hlen = prf ? SHA512_DIGEST_LENGTH : SHA256_DIGEST_LENGTH;
    size_t need, off;
    unsigned ctr;
    if (lbits == 0 || lbits % 8) return -1;
    need = lbits / 8;
    if (need > MAXBUF) return -1;
    off = 0;
    for (ctr = 1; off < need; ctr++) {
        unsigned char block[128];
        unsigned int blen = 0;
        unsigned char cbe[4];
        unsigned char cbits[4];
        unsigned char lbe[4];
        /* BoringSSL: stack context, no _new/_free (mirrors HashPassword). */
        HMAC_CTX h;
        cbe[0] = (ctr >> 24) & 0xff;
        cbe[1] = (ctr >> 16) & 0xff;
        cbe[2] = (ctr >> 8) & 0xff;
        cbe[3] = ctr & 0xff;
        HMAC_CTX_init(&h);
        /* Key = current input bytes (variable length, like HashPassword). */
        if (!HMAC_Init_ex(&h, g_buf, (int)g_len, md, NULL)) return -1;
        HMAC_Update(&h, cbe, 4);
        HMAC_Update(&h, (const unsigned char*)label, strlen(label));
        HMAC_Update(&h, (const unsigned char*)"\0", 1);
        HMAC_Update(&h, (const unsigned char*)ctx, strlen(ctx));
        {
            unsigned cb = (unsigned)(strlen(ctx) * 8);
            cbits[0] = (cb >> 24) & 0xff;
            cbits[1] = (cb >> 16) & 0xff;
            cbits[2] = (cb >> 8) & 0xff;
            cbits[3] = cb & 0xff;
        }
        HMAC_Update(&h, cbits, 4);
        lbe[0] = (lbits >> 24) & 0xff;
        lbe[1] = (lbits >> 16) & 0xff;
        lbe[2] = (lbits >> 8) & 0xff;
        lbe[3] = lbits & 0xff;
        HMAC_Update(&h, lbe, 4);
        if (!HMAC_Final(&h, block, &blen)) return -1;
        {
            size_t take = need - off < hlen ? need - off : hlen;
            memcpy(out + off, block, take);
            off += take;
        }
    }
    *outlen = need;
    return 0;
}

static int run_stage(char* argv[], int argc) {
    unsigned char out[MAXBUF];
    size_t outlen = 0;
    if (argc < 1) return -1;
    if (cmp_label(argv[0], "slice") && argc == 3) {
        long a = strtol(argv[1], NULL, 10);
        long b = strtol(argv[2], NULL, 10);
        if (a < 0 || b < a || (size_t)b > g_len) return -1;
        outlen = (size_t)(b - a);
        memcpy(out, g_buf + a, outlen);
    } else if (cmp_label(argv[0], "hex") && argc == 1) {
        /* hex stays text: emit directly, bypass binary buffer. */
        size_t i;
        for (i = 0; i < g_len; i++) printf("%02x", g_buf[i]);
        printf("\n");
        /* Signal straight-to-stdout: caller skips the final emit. */
        g_len = (size_t)-1;
        return 0;
    } else if (cmp_label(argv[0], "unhex") && argc == 1) {
        /* Current bytes are hex text. */
        char tmp[MAXBUF * 2 + 1];
        if (g_len >= sizeof(tmp)) return -1;
        memcpy(tmp, g_buf, g_len);
        tmp[g_len] = '\0';
        {
            int n = from_hex(tmp, out, sizeof(out));
            if (n < 0) return -1;
            outlen = (size_t)n;
        }
    } else if (cmp_label(argv[0], "ph512") && (argc == 1 || argc == 2)) {
        if (do_ph(argc == 2 ? argv[1] : "fbe-key", 0, out, &outlen)) return -1;
    } else if (cmp_label(argv[0], "ph256") && (argc == 1 || argc == 2)) {
        if (do_ph(argc == 2 ? argv[1] : "fbe-key", 1, out, &outlen)) return -1;
    } else if (cmp_label(argv[0], "sp800") && argc == 5) {
        int prf;
        unsigned lbits;
        if (cmp_label(argv[3], "sha256"))
            prf = 0;
        else if (cmp_label(argv[3], "sha512"))
            prf = 1;
        else
            return -1;
        lbits = (unsigned)strtoul(argv[4], NULL, 10);
        if (do_sp800(argv[1], argv[2], prf, lbits, out, &outlen)) return -1;
    } else if (cmp_label(argv[0], "sha256") && argc == 1) {
        SHA256(g_buf, g_len, out);
        outlen = SHA256_DIGEST_LENGTH;
    } else if (cmp_label(argv[0], "sha512") && argc == 1) {
        SHA512(g_buf, g_len, out);
        outlen = SHA512_DIGEST_LENGTH;
    } else if (cmp_label(argv[0], "xorhalf") && argc == 1) {
        size_t i, h;
        if ((g_len & 1) || g_len == 0) return -1;
        h = g_len / 2;
        for (i = 0; i < h; i++) out[i] = g_buf[i] ^ g_buf[i + h];
        outlen = h;
    } else {
        return -2;
    }
    if (outlen > sizeof(g_buf)) return -1;
    memcpy(g_buf, out, outlen);
    g_len = outlen;
    return 0;
}

int main(int argc, char* argv[]) {
    char* expr;
    char* stages[MAXSTAGES];
    int nstages = 0;
    int i;
    int secret_len;
    if (argc != 3) {
        fprintf(stderr, "usage: %s <secret-hex> \"<stage> [args] | ...\"\n", argv[0]);
        return 1;
    }
    secret_len = from_hex(argv[1], g_buf, sizeof(g_buf));
    if (secret_len < 0) {
        fprintf(stderr, "bad secret hex\n");
        return 1;
    }
    g_len = (size_t)secret_len;
    expr = argv[2];
    /* Split pipeline on '|'. */
    stages[nstages++] = expr;
    for (i = 0; expr[i]; i++) {
        if (expr[i] == '|') {
            expr[i] = '\0';
            if (nstages >= MAXSTAGES) {
                fprintf(stderr, "too many stages\n");
                return 1;
            }
            stages[nstages++] = expr + i + 1;
        }
    }
    for (i = 0; i < nstages; i++) {
        char* tok[MAXARGS];
        int ntok = 0;
        char* p = stages[i];
        /* Trim + tokenize on spaces/tabs. */
        while (*p == ' ' || *p == '\t') p++;
        {
            char* end = p + strlen(p);
            while (end > p && (end[-1] == ' ' || end[-1] == '\t')) *--end = '\0';
        }
        while (*p && ntok < MAXARGS) {
            tok[ntok++] = p;
            while (*p && *p != ' ' && *p != '\t') p++;
            if (*p) *p++ = '\0';
            while (*p == ' ' || *p == '\t') p++;
        }
        if (ntok == 0) continue;
        {
            int rc = run_stage(tok, ntok);
            if (rc == -2) {
                fprintf(stderr, "unknown stage: %s\n", tok[0]);
                return 2;
            }
            if (rc) {
                fprintf(stderr, "stage failed: %s\n", tok[0]);
                return 1;
            }
        }
        if (g_len == (size_t)-1) return 0; /* hex emitted directly */
    }
    /* Final emit: hex of current bytes. */
    {
        size_t k;
        for (k = 0; k < g_len; k++) printf("%02x", g_buf[k]);
        printf("\n");
    }
    return 0;
}
