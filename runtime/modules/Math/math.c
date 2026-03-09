#include <math.h>
#include <stdbool.h>
#include <stdlib.h>

static const double CB_MATH_PI = 3.14159265358979323846;
static const double CB_MATH_E = 2.71828182845904523536;

long long math_max_i64(long long a, long long b) {
    return a > b ? a : b;
}

double math_max_f64(double a, double b) {
    return fmax(a, b);
}

long long math_min_i64(long long a, long long b) {
    return a < b ? a : b;
}

double math_min_f64(double a, double b) {
    return fmin(a, b);
}

double math_euler(void) {
    return CB_MATH_E;
}

double math_pi(void) {
    return CB_MATH_PI;
}

long long math_abs_i64(long long value) {
    return llabs(value);
}

double math_abs_f64(double value) {
    return fabs(value);
}

long long math_sign_i64(long long value) {
    return (value > 0) - (value < 0);
}

long long math_sign_f64(double value) {
    if (isnan(value)) {
        return 0;
    }
    return (value > 0.0) - (value < 0.0);
}

double math_sqrt_i64(long long value) {
    return sqrt((double)value);
}

double math_sqrt_f64(double value) {
    return sqrt(value);
}

double math_cbrt_i64(long long value) {
    return cbrt((double)value);
}

double math_cbrt_f64(double value) {
    return cbrt(value);
}

double math_pow_i64_i64(long long base, long long exponent) {
    return pow((double)base, (double)exponent);
}

double math_pow_f64_f64(double base, double exponent) {
    return pow(base, exponent);
}

double math_pow_i64_f64(long long base, double exponent) {
    return pow((double)base, exponent);
}

double math_pow_f64_i64(double base, long long exponent) {
    return pow(base, (double)exponent);
}

double math_exp_i64(long long value) {
    return exp((double)value);
}

double math_exp_f64(double value) {
    return exp(value);
}

double math_log_i64(long long value) {
    return log((double)value);
}

double math_log_f64(double value) {
    return log(value);
}

double math_log10_i64(long long value) {
    return log10((double)value);
}

double math_log10_f64(double value) {
    return log10(value);
}

double math_log2_i64(long long value) {
    return log2((double)value);
}

double math_log2_f64(double value) {
    return log2(value);
}

double math_floor_i64(long long value) {
    return floor((double)value);
}

double math_floor_f64(double value) {
    return floor(value);
}

double math_ceil_i64(long long value) {
    return ceil((double)value);
}

double math_ceil_f64(double value) {
    return ceil(value);
}

double math_round_i64(long long value) {
    return round((double)value);
}

double math_round_f64(double value) {
    return round(value);
}

double math_trunc_i64(long long value) {
    return trunc((double)value);
}

double math_trunc_f64(double value) {
    return trunc(value);
}

double math_sin_i64(long long radians) {
    return sin((double)radians);
}

double math_sin_f64(double radians) {
    return sin(radians);
}

double math_cos_i64(long long radians) {
    return cos((double)radians);
}

double math_cos_f64(double radians) {
    return cos(radians);
}

double math_tan_i64(long long radians) {
    return tan((double)radians);
}

double math_tan_f64(double radians) {
    return tan(radians);
}

double math_asin_i64(long long value) {
    return asin((double)value);
}

double math_asin_f64(double value) {
    return asin(value);
}

double math_acos_i64(long long value) {
    return acos((double)value);
}

double math_acos_f64(double value) {
    return acos(value);
}

double math_atan_i64(long long value) {
    return atan((double)value);
}

double math_atan_f64(double value) {
    return atan(value);
}

double math_atan2_i64_i64(long long y, long long x) {
    return atan2((double)y, (double)x);
}

double math_atan2_f64_f64(double y, double x) {
    return atan2(y, x);
}

double math_atan2_i64_f64(long long y, double x) {
    return atan2((double)y, x);
}

double math_atan2_f64_i64(double y, long long x) {
    return atan2(y, (double)x);
}

double math_hypot_i64_i64(long long a, long long b) {
    return hypot((double)a, (double)b);
}

double math_hypot_f64_f64(double a, double b) {
    return hypot(a, b);
}

double math_hypot_i64_f64(long long a, double b) {
    return hypot((double)a, b);
}

double math_hypot_f64_i64(double a, long long b) {
    return hypot(a, (double)b);
}

long long math_clamp_i64(long long value, long long lower, long long upper) {
    if (lower > upper) {
        long long temp = lower;
        lower = upper;
        upper = temp;
    }

    if (value < lower) {
        return lower;
    }
    if (value > upper) {
        return upper;
    }
    return value;
}

double math_clamp_f64(double value, double lower, double upper) {
    if (lower > upper) {
        double temp = lower;
        lower = upper;
        upper = temp;
    }

    if (value < lower) {
        return lower;
    }
    if (value > upper) {
        return upper;
    }
    return value;
}

double math_deg2rad_i64(long long degrees) {
    return ((double)degrees) * (CB_MATH_PI / 180.0);
}

double math_deg2rad_f64(double degrees) {
    return degrees * (CB_MATH_PI / 180.0);
}

double math_rad2deg_i64(long long radians) {
    return ((double)radians) * (180.0 / CB_MATH_PI);
}

double math_rad2deg_f64(double radians) {
    return radians * (180.0 / CB_MATH_PI);
}

bool math_is_nan_f64(double value) {
    return isnan(value);
}

bool math_is_inf_f64(double value) {
    return isinf(value);
}
