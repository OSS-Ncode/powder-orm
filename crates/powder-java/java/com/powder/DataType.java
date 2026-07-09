package com.powder;

/** The four physical column types carried by the PCB wire format. */
public enum DataType {
    INT64,
    FLOAT64,
    BOOL,
    UTF8;

    static DataType fromCode(int code) {
        switch (code) {
            case 0:
                return INT64;
            case 1:
                return FLOAT64;
            case 2:
                return BOOL;
            case 3:
                return UTF8;
            default:
                throw new IllegalArgumentException("unsupported PCB type code " + code);
        }
    }
}
