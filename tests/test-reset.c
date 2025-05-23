/**
 * test-reset.c
 *
 * Copyright (c) 2012
 *      libchewing Core Team.
 *
 * See the file "COPYING" for information on usage and redistribution
 * of this file.
 */

#ifdef HAVE_CONFIG_H
#    include <config.h>
#endif

#include <assert.h>
#include <stdlib.h>

#include "chewing.h"
#include "testhelper.h"

void test_reset_shall_not_clean_static_data()
{
    const TestData DATA = { "hk4g4<E>", "\xE6\xB8\xAC\xE8\xA9\xA6" /* 測試 */  };
    ChewingContext *ctx;

    ctx = chewing_new();
    start_testcase(ctx);

    chewing_set_KBType(ctx, chewing_KBStr2Num("KB_DEFAULT"));

    chewing_set_maxChiSymbolLen(ctx, 16);

    chewing_Reset(ctx);

    type_keystroke_by_string(ctx, DATA.token);
    ok_commit_buffer(ctx, DATA.expected);

    chewing_delete(ctx);
}

int main(int argc, char *argv[])
{
    putenv("CHEWING_PATH=" CHEWING_DATA_PREFIX);
    putenv("CHEWING_USER_PATH=" TEST_HASH_DIR);

    test_reset_shall_not_clean_static_data();

    return exit_status();
}
