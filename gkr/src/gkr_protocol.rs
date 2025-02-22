use crate::gkr_circuit::{Circuit, Layer, Operation};
use univariate_polynomial::univariate_polynomial_dense::UnivariatePoly;

use ark_bn254::Fq;

use fiat_shamir::fiat_shamir_transcript::{fq_vec_to_bytes, Transcript};
use multilinear_polynomial::{
    composed_polynomial::{ProductPoly, SumPoly},
    multilinear_polynomial_evaluation::MultilinearPoly,
};
use sum_check::sum_check_protocol::{gkr_prove, gkr_verify};

pub struct Proof {
    output_poly: MultilinearPoly<Fq>,
    proof_polynomials: Vec<Vec<UnivariatePoly<Fq>>>,
    claimed_evaluations: Vec<(Fq, Fq)>,
}

pub fn prove(circuit: &mut Circuit<Fq>, inputs: &[Fq]) -> Proof {
    let mut transcript = Transcript::<Fq>::new();

    let mut circuit_evaluations = circuit.evaluate(inputs);

    let mut w_0 = circuit_evaluations.last().unwrap().to_vec();

    if w_0.len() == 1 {
        w_0.push(Fq::from(0));
    }

    let output_poly = MultilinearPoly::new(w_0);

    let (mut claimed_sum, mut random_challenge) = initiate_protocol(&mut transcript, &output_poly);

    let num_rounds = 2;
    let mut proof_polys = Vec::with_capacity(num_rounds);
    let mut claimed_evaluations: Vec<(Fq, Fq)> = Vec::new();

    let mut current_alpha = Fq::from(0);
    let mut current_beta = Fq::from(0);
    let mut current_rb = Vec::new();
    let mut current_rc = Vec::new();

    circuit_evaluations.reverse();

    for idx in 0..circuit.layers.len() {
        let mut layers = circuit.layers.clone();
        layers.reverse();

        let w_i = if idx == circuit.layers.len() - 1 {
            inputs.to_vec()
        } else {
            circuit_evaluations[idx + 1].clone()
        };

        let fbc_poly: SumPoly<Fq> = if idx == 0 {
            get_fbc_poly(random_challenge, layers[idx].clone(), &w_i, &w_i)
        } else {
            get_merged_fbc_poly(
                layers[idx].clone(),
                &w_i,
                &w_i,
                &current_rb,
                &current_rc,
                current_alpha,
                current_beta,
            )
        };

        let sum_check_proof = gkr_prove(claimed_sum, &fbc_poly, &mut transcript);

        proof_polys.push(sum_check_proof.proof_polynomials);

        let next_poly = MultilinearPoly::new(w_i);

        let (r_b, r_c) = sum_check_proof
            .random_challenges
            .split_at(sum_check_proof.random_challenges.len() / 2);

        let o_1 = next_poly.evaluate(r_b.to_vec());
        let o_2 = next_poly.evaluate(r_c.to_vec());

        current_rb = r_b.to_vec();
        current_rc = r_c.to_vec();

        transcript.append(&fq_vec_to_bytes(&[o_1]));
        current_alpha = transcript.get_random_challenge();

        transcript.append(&fq_vec_to_bytes(&[o_2]));
        current_beta = transcript.get_random_challenge();

        claimed_sum = (current_alpha * o_1) + (current_beta * o_2);

        claimed_evaluations.push((o_1, o_2));

        random_challenge = transcript.get_random_challenge();
    }

    Proof {
        output_poly,
        proof_polynomials: proof_polys,
        claimed_evaluations,
    }
}

pub fn verify(proof: Proof, mut circuit: Circuit<Fq>, inputs: &[Fq]) -> bool {
    let mut transcript = Transcript::<Fq>::new();

    let (mut current_claim, mut current_random_challenge) =
        initiate_protocol(&mut transcript, &proof.output_poly);

    let mut sumcheck_random_challenges = Vec::new();
    let mut current_alpha = Fq::from(0);
    let mut current_beta = Fq::from(0);

    circuit.layers.reverse();

    for i in 0..circuit.layers.len() {
        let sum_check_verify = gkr_verify(
            proof.proof_polynomials[i].clone(),
            current_claim,
            &mut transcript,
        );

        if !sum_check_verify.verified {
            return false;
        }

        let layer = &circuit.layers[i];

        let (o_1, o_2) = proof.claimed_evaluations[i];

        let (claim, next_claim, alpha, beta) = get_verifier_claim(
            layer,
            current_random_challenge,
            &sum_check_verify.random_challenges,
            o_1,
            o_2,
            &mut transcript,
            i,
        );

        if claim == sum_check_verify.final_claimed_sum {
            println!("check on layer {i} passed!");
            current_claim = next_claim;
            current_random_challenge = transcript.get_random_challenge();
            sumcheck_random_challenges.push(sum_check_verify.random_challenges);
            current_alpha = alpha;
            current_beta = beta;
        } else {
            return false;
        }
    }

    //todo run a simple test to check all the expected claimed sums

    let r_count = sumcheck_random_challenges.len();

    let (r_1, r_2) = sumcheck_random_challenges[r_count - 2]
        .split_at(sumcheck_random_challenges[r_count - 2].len() / 2);

    let input_fbc_poly = get_merged_fbc_poly(
        circuit.layers.last().unwrap().clone(),
        inputs,
        inputs,
        r_1,
        r_2,
        current_alpha,
        current_beta,
    );

    let input_layer_claim =
        input_fbc_poly.evaluate(sumcheck_random_challenges.last().unwrap().to_vec());

    println!(
        "final claim is {:?} and input claim is {:?}",
        current_claim, input_layer_claim
    );

    input_layer_claim == current_claim
}

fn initiate_protocol(
    transcript: &mut Transcript<Fq>,
    output_poly: &MultilinearPoly<Fq>,
) -> (Fq, Fq) {
    transcript.append(&fq_vec_to_bytes(&output_poly.evaluation));

    let random_challenge = transcript.get_random_challenge();

    let m_0 = output_poly.evaluate(vec![random_challenge]);

    transcript.append(&fq_vec_to_bytes(&[m_0]));

    (m_0, random_challenge)
}

fn add_mul_polynomials(poly_a: &[Fq], poly_b: &[Fq], op: Operation) -> MultilinearPoly<Fq> {
    let new_eval_len = poly_a.len() * poly_b.len();
    let mut new_eval = Vec::with_capacity(new_eval_len);

    for a in poly_a {
        for b in poly_b {
            new_eval.push(op.apply(*a, *b))
        }
    }

    MultilinearPoly::new(new_eval)
}

fn get_fbc_poly(random_challenge: Fq, layer: Layer<Fq>, w_b: &[Fq], w_c: &[Fq]) -> SumPoly<Fq> {
    let add_i = layer
        .get_add_mul_i(Operation::Add)
        .partial_evaluate(0, &random_challenge);
    let mul_i = layer
        .get_add_mul_i(Operation::Mul)
        .partial_evaluate(0, &random_challenge);

    let summed_w_poly = add_mul_polynomials(w_b, w_c, Operation::Add);
    let multiplied_w_poly = add_mul_polynomials(w_b, w_c, Operation::Mul);

    let add_w_eval = vec![add_i.evaluation, summed_w_poly.evaluation];
    let mul_w_eval = vec![mul_i.evaluation, multiplied_w_poly.evaluation];

    let add_eval_product = ProductPoly::new(add_w_eval);
    let mul_eval_product = ProductPoly::new(mul_w_eval);

    SumPoly::new(vec![add_eval_product, mul_eval_product])
}

fn get_merged_fbc_poly(
    layer: Layer<Fq>,
    w_b: &[Fq],
    w_c: &[Fq],
    r_b: &[Fq],
    r_c: &[Fq],
    alpha: Fq,
    beta: Fq,
) -> SumPoly<Fq> {
    let add_i = layer.get_add_mul_i(Operation::Add);
    let mul_i = layer.get_add_mul_i(Operation::Mul);

    let add_i_rb = add_i.multi_partial_evaluate(r_b).scale(alpha);
    let add_i_rc = add_i.multi_partial_evaluate(r_c).scale(beta);

    let mul_i_rb = mul_i.multi_partial_evaluate(r_b).scale(alpha);
    let mul_i_rc = mul_i.multi_partial_evaluate(r_c).scale(beta);

    let summed_w_poly = add_mul_polynomials(w_b, w_c, Operation::Add);
    let multiplied_w_poly = add_mul_polynomials(w_b, w_c, Operation::Mul);

    let summed_add_i = add_i_rb.clone() + add_i_rc.clone();
    let summed_mul_i = mul_i_rb + mul_i_rc;

    let add_product_poly = ProductPoly::new(vec![
        summed_add_i.evaluation,
        summed_w_poly.evaluation.clone(),
    ]);
    let mul_product_poly = ProductPoly::new(vec![
        summed_mul_i.evaluation,
        multiplied_w_poly.evaluation.clone(),
    ]);

    SumPoly::new(vec![add_product_poly, mul_product_poly])
}

fn get_verifier_claim(
    layer: &Layer<Fq>,
    init_random_challenge: Fq,
    sumcheck_random_challenges: &[Fq],
    o_1: Fq,
    o_2: Fq,
    transcript: &mut Transcript<Fq>,
    layer_id: usize,
) -> (Fq, Fq, Fq, Fq) {
    let (r_b, r_c) = sumcheck_random_challenges.split_at(sumcheck_random_challenges.len() / 2);

    let mut all_random_challenges = Vec::with_capacity(1 + r_b.len() + r_c.len());

    all_random_challenges.push(init_random_challenge);
    all_random_challenges.extend_from_slice(r_b);
    all_random_challenges.extend_from_slice(r_c);

    let a_r = layer
        .get_add_mul_i(Operation::Add)
        .evaluate(all_random_challenges.clone());

    let m_r = layer
        .get_add_mul_i(Operation::Mul)
        .evaluate(all_random_challenges);

    transcript.append(&fq_vec_to_bytes(&[o_1]));
    let alpha = transcript.get_random_challenge();

    transcript.append(&fq_vec_to_bytes(&[o_2]));
    let beta = transcript.get_random_challenge();

    let current_claim = if layer_id == 0 {
        a_r * (o_1 + o_2) + m_r * (o_1 * o_2)
    } else {
        (alpha * a_r * (o_1 + o_2)) + (beta * m_r * (o_1 * o_2))
    };

    //? maybe dont yet increase till you confirm its verified?
    let next_claim = (alpha * o_1) + (beta * o_2);

    (current_claim, next_claim, alpha, beta)
}

#[cfg(test)]
mod test {
    use super::{add_mul_polynomials, get_fbc_poly, prove, verify, Proof};
    use crate::gkr_circuit::{Circuit, Operation};
    use ark_bn254::Fq;
    use multilinear_polynomial::{
        composed_polynomial::{ProductPoly, SumPoly},
        multilinear_polynomial_evaluation::MultilinearPoly,
    };
    use univariate_polynomial::univariate_polynomial_dense::UnivariatePoly;

    #[test]
    fn it_add_polys_correctly() {
        let poly_a = &[Fq::from(0), Fq::from(2)];
        let poly_b = &[Fq::from(0), Fq::from(3)];

        let expected_poly = vec![Fq::from(0), Fq::from(3), Fq::from(2), Fq::from(5)];

        let result = add_mul_polynomials(poly_a, poly_b, Operation::Add);

        assert_eq!(result.evaluation, expected_poly);

        let poly_a = &[Fq::from(0), Fq::from(3)];
        let poly_b = &[Fq::from(0), Fq::from(0), Fq::from(0), Fq::from(2)];

        let expected_poly = vec![
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(2),
            Fq::from(3),
            Fq::from(3),
            Fq::from(3),
            Fq::from(5),
        ];

        let result = add_mul_polynomials(poly_a, poly_b, Operation::Add);

        assert_eq!(result.evaluation, expected_poly);
    }

    #[test]
    fn it_multiplies_polys_correctly() {
        let poly_a = &[Fq::from(0), Fq::from(2)];
        let poly_b = &[Fq::from(0), Fq::from(3)];

        let expected_poly = vec![Fq::from(0), Fq::from(0), Fq::from(0), Fq::from(6)];

        let result = add_mul_polynomials(poly_a, poly_b, Operation::Mul);

        assert_eq!(result.evaluation, expected_poly);

        let poly_a = &[Fq::from(0), Fq::from(3)];
        let poly_b = &[Fq::from(0), Fq::from(0), Fq::from(0), Fq::from(2)];

        let expected_poly = vec![
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(6),
        ];

        let result = add_mul_polynomials(poly_a, poly_b, Operation::Mul);

        assert_eq!(result.evaluation, expected_poly);
    }

    #[test]
    fn test_get_fbc_poly() {
        // let r_c = Fq::from(5);
        // let add_i = MultilinearPoly::new(vec![
        //     Fq::from(0),
        //     Fq::from(1),
        //     Fq::from(0),
        //     Fq::from(0),
        //     Fq::from(0),
        //     Fq::from(0),
        //     Fq::from(0),
        //     Fq::from(0),
        // ]);
        // let mul_i = MultilinearPoly::new(vec![Fq::from(0); 8]);
        // let w_1_poly = &[Fq::from(2), Fq::from(12)];

        // let fbc_poly = get_fbc_poly(r_c, add_i.clone(), mul_i.clone(), w_1_poly, w_1_poly);

        // let one = ProductPoly::new(vec![
        //     add_i.evaluation,
        //     vec![Fq::from(4), Fq::from(14), Fq::from(14), Fq::from(24)],
        // ]);
        // let two = ProductPoly::new(vec![
        //     mul_i.evaluation,
        //     vec![Fq::from(4), Fq::from(24), Fq::from(24), Fq::from(144)],
        // ]);

        // let expected_result = SumPoly::new(vec![one, two]);

        // assert_eq!(fbc_poly.polys, expected_result.polys);
    }

    #[test]
    fn test_get_merged_fbc_poly() {}

    #[test]
    fn test_valid_proving_and_verification() {
        let circuit_structure: Vec<Vec<Operation>> = vec![
            vec![
                Operation::Mul,
                Operation::Mul,
                Operation::Mul,
                Operation::Mul,
            ],
            vec![Operation::Add, Operation::Add],
            vec![Operation::Add],
        ];

        let inputs: Vec<Fq> = vec![
            Fq::from(5),
            Fq::from(2),
            Fq::from(2),
            Fq::from(4),
            Fq::from(10),
            Fq::from(0),
            Fq::from(3),
            Fq::from(3),
        ];

        let mut circuit = Circuit::new(circuit_structure);

        let proof = prove(&mut circuit, &inputs);

        let is_verified = verify(proof, circuit, &inputs);

        assert_eq!(is_verified, true);
    }

    #[test]
    fn test_verify_invalid_proof() {
        let circuit_structure: Vec<Vec<Operation>> = vec![
            vec![
                Operation::Mul,
                Operation::Mul,
                Operation::Mul,
                Operation::Mul,
            ],
            vec![Operation::Add, Operation::Add],
            vec![Operation::Add],
        ];

        let inputs: Vec<Fq> = vec![
            Fq::from(5),
            Fq::from(2),
            Fq::from(2),
            Fq::from(4),
            Fq::from(10),
            Fq::from(0),
            Fq::from(3),
            Fq::from(3),
        ];

        let circuit = Circuit::new(circuit_structure);

        let dummy_proof_poly_1 = UnivariatePoly::new(vec![Fq::from(10), Fq::from(5)]);
        let dummy_proof_poly_2 = UnivariatePoly::new(vec![Fq::from(10), Fq::from(5)]);
        let dummy_proof_poly_3 = UnivariatePoly::new(vec![Fq::from(10), Fq::from(5)]);
        let dummy_proof_poly_4 = UnivariatePoly::new(vec![Fq::from(10), Fq::from(5)]);

        let invalid_proof = Proof {
            output_poly: MultilinearPoly::new(vec![Fq::from(10)]),
            proof_polynomials: vec![
                vec![dummy_proof_poly_1, dummy_proof_poly_2],
                vec![dummy_proof_poly_3, dummy_proof_poly_4],
            ],
            claimed_evaluations: vec![(Fq::from(10), Fq::from(5)), (Fq::from(1), Fq::from(2))],
        };

        let is_verified = verify(invalid_proof, circuit, &inputs);

        assert_eq!(is_verified, false);
    }
}
