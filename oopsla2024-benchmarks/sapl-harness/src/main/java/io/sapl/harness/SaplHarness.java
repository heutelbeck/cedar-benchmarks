package io.sapl.harness;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

import io.sapl.api.model.ObjectValue;
import io.sapl.api.model.Value;
import io.sapl.api.model.ValueJsonMarshaller;
import io.sapl.api.model.jackson.SaplJacksonModule;
import io.sapl.api.pdp.AuthorizationSubscription;
import io.sapl.api.pdp.Decision;
import io.sapl.api.pdp.configuration.CombiningAlgorithm;
import io.sapl.api.pdp.configuration.CombiningAlgorithm.DefaultDecision;
import io.sapl.api.pdp.configuration.CombiningAlgorithm.ErrorHandling;
import io.sapl.api.pdp.configuration.CombiningAlgorithm.VotingMode;
import io.sapl.api.pdp.configuration.PDPConfiguration;
import io.sapl.api.pdp.configuration.PdpData;
import io.sapl.pdp.PDPComponents;
import io.sapl.pdp.PolicyDecisionPointBuilder;
import tools.jackson.databind.JsonNode;
import tools.jackson.databind.json.JsonMapper;

/**
 * Thin Java harness for SAPL evaluation in the Cedar OOPSLA benchmark suite.
 * <p>
 * Protocol: reads line-delimited JSON from stdin, one line per hierarchy batch.
 * Each batch contains variables and subscriptions. Policies are loaded from disk
 * at startup. Writes one JSON line per batch to stdout with per-request timing.
 * <p>
 * Usage: java -jar sapl-harness.jar --app github --policies-dir ./policies/github
 */
public class SaplHarness {

    private static final JsonMapper MAPPER = JsonMapper.builder().addModule(new SaplJacksonModule()).build();

    private static final ObjectValue COMPILER_OPTIONS = (ObjectValue) ValueJsonMarshaller.json("""
            { "indexing": "SMTDD" }
            """);

    public static void main(String[] args) throws Exception {
        var app      = parseApp(args);
        var policies = loadPoliciesFromResources(app);

        System.err.println("SAPL harness ready: " + policies.size() + " policies for " + app);

        var reader     = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        var firstBatch = true;

        String line;
        while ((line = reader.readLine()) != null) {
            if (line.isBlank()) {
                continue;
            }
            var batch   = MAPPER.readTree(line);
            var results = processBatch(batch, policies, firstBatch);
            firstBatch = false;
            System.out.println(MAPPER.writeValueAsString(results));
            System.out.flush();
        }
    }

    private static List<TestOutput> processBatch(JsonNode batch, List<String> policies, boolean warmup) {
        var variables     = extractVariables(batch);
        var subscriptions = extractSubscriptions(batch);

        PDPComponents components = null;
        try {
            var pdpData          = new PdpData(variables, Value.EMPTY_OBJECT);
            var pdpConfiguration = new PDPConfiguration("default", "cedar-bench", ALGORITHM, COMPILER_OPTIONS,
                    policies, pdpData);
            components = PolicyDecisionPointBuilder.withDefaults().withConfiguration(pdpConfiguration).build();
            var pdp = components.pdp();

            // Warmup first batch for 15s to trigger JIT compilation
            if (warmup) {
                System.err.println("Warming up JIT for 15s...");
                var warmupDeadline = System.nanoTime() + 15_000_000_000L;
                while (System.nanoTime() < warmupDeadline) {
                    for (var sub : subscriptions) {
                        pdp.decideOnce(sub);
                    }
                }
                System.err.println("Warmup complete.");
            }

            // Timed evaluation
            var results = new ArrayList<TestOutput>(subscriptions.size());
            for (var sub : subscriptions) {
                var start    = System.nanoTime();
                var decision = pdp.decideOnce(sub);
                var dur      = System.nanoTime() - start;
                results.add(new TestOutput(decision.decision() == Decision.PERMIT, dur));
            }
            return results;
        } finally {
            if (components != null) {
                components.close();
            }
        }
    }

    private static ObjectValue extractVariables(JsonNode batch) {
        var variablesNode = batch.get("variables");
        if (variablesNode == null || variablesNode.isNull()) {
            return Value.EMPTY_OBJECT;
        }
        return (ObjectValue) ValueJsonMarshaller.json(variablesNode.toString());
    }

    private static final CombiningAlgorithm ALGORITHM = new CombiningAlgorithm(
            VotingMode.PRIORITY_DENY, DefaultDecision.DENY, ErrorHandling.PROPAGATE);

    private static List<AuthorizationSubscription> extractSubscriptions(JsonNode batch) {
        var subsNode      = batch.get("subscriptions");
        var subscriptions = new ArrayList<AuthorizationSubscription>();
        if (subsNode != null && subsNode.isArray()) {
            for (var subNode : subsNode) {
                subscriptions.add(MAPPER.treeToValue(subNode, AuthorizationSubscription.class));
            }
        }
        return subscriptions;
    }

    private static List<String> loadPoliciesFromResources(String app) throws Exception {
        var policies = new ArrayList<String>();
        var classLoader = SaplHarness.class.getClassLoader();
        for (int i = 1; i <= 20; i++) {
            var name = app + "/policy-" + String.format("%04d", i) + ".sapl";
            try (var stream = classLoader.getResourceAsStream(name)) {
                if (stream == null) {
                    break;
                }
                policies.add(new String(stream.readAllBytes(), StandardCharsets.UTF_8));
            }
        }
        if (policies.isEmpty()) {
            System.err.println("No policies found for app: " + app);
            System.exit(1);
        }
        return policies;
    }

    private static String parseApp(String[] args) {
        for (int i = 0; i < args.length - 1; i++) {
            if ("--app".equals(args[i])) {
                return args[i + 1];
            }
        }
        System.err.println("Usage: java -jar sapl-harness.jar --app <github|gdrive|tinytodo>");
        System.exit(1);
        return null;
    }

    record TestOutput(boolean Decision, long Dur) {}
}
