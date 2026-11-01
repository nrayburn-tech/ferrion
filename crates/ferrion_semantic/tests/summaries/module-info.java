import com.example.spi.Service;

/** A module. */
@Deprecated
open module com.example.app {
    requires java.base;
    requires transitive java.sql;
    requires static java.compiler;
    requires static transitive org.slf4j;
    exports com.example.api;
    exports com.example.internal to com.example.test, com.example.tools;
    opens com.example.model;
    opens com.example.data to com.fasterxml.jackson.databind;
    uses com.example.spi.Service;
    provides com.example.spi.Service with com.example.impl.ServiceImpl, com.example.impl.Other;
}
