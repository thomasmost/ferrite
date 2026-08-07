/* Wrapper for Pebble SDK's pebble.h to work around tm redefinition issues.
   This file must be in the include path BEFORE the SDK's pebble.h.

   The issue: pebble.h extends struct tm (from time.h) with extra fields
   (tm_gmtoff, tm_zone), but can't redefine it after time.h has defined it.
   Solution: Define struct tm and all time.h types BEFORE pebble.h is included,
   then when pebble.h redefines it, it's a benign redefinition of an identical struct.
*/

#pragma once
#ifndef __PEBBLE_H__
#define __PEBBLE_H__

/* Prevent time.h from being included by pebble.h by pre-defining everything it needs. */
#define _TIME_H_
#define _SYS_TIME_H_

/* Minimal time.h type definitions needed by pebble.h */
typedef long time_t;
typedef long clock_t;

#define TZ_LEN 6
#define CLOCKS_PER_SEC 1000000

struct tm {
    int tm_sec;      /* Seconds. [0-60] (1 leap second) */
    int tm_min;      /* Minutes. [0-59] */
    int tm_hour;     /* Hours.  [0-23] */
    int tm_mday;     /* Day. [1-31] */
    int tm_mon;      /* Month. [0-11] */
    int tm_year;     /* Years since 1900 */
    int tm_wday;     /* Day of week. [0-6] */
    int tm_yday;     /* Days in year.[0-365] */
    int tm_isdst;    /* DST. [-1/0/1] */
    int tm_gmtoff;   /* Seconds east of UTC (Pebble extension) */
    char tm_zone[TZ_LEN]; /* Timezone abbreviation (Pebble extension) */
};

#endif

/* Now include the real pebble.h. Its struct tm definition will be identical
   to ours (since we included all the extended fields), so the redefinition is benign. */
#include_next <pebble.h>
