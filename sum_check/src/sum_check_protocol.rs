use ark_bn254::Fq;
use fiat_shamir::fiat_shamir_transcript::{fq_vec_to_bytes, Transcript};
use multilinear_polynomial::{
    composed_polynomial::SumPoly, multilinear_polynomial_evaluation::MultilinearPoly,
};
use univariate_polynomial::univariate_polynomial_dense::UnivariatePoly;

#[derive(Debug, Clone)]
pub struct Proof {
    pub proof_polynomials: Vec<Vec<Fq>>,
    pub claimed_sum: Fq,
}
pub struct GkrProof {
    pub proof_polynomials: Vec<UnivariatePoly<Fq>>,
    pub claimed_sum: Fq,
    pub random_challenges: Vec<Fq>,
}

pub struct GkrVerify {
    pub verified: bool,
    pub final_claimed_sum: Fq,
    pub random_challenges: Vec<Fq>,
}

pub fn prove(polynomial: &MultilinearPoly<Fq>) -> Proof {
    let mut transcript = Transcript::<Fq>::new();
    transcript.append(&fq_vec_to_bytes(&polynomial.evaluation));

    let claimed_sum: Fq = polynomial.evaluation.iter().sum();
    transcript.append(&fq_vec_to_bytes(&[claimed_sum]));

    let num_rounds = polynomial.num_of_vars;
    let mut proof_polynomials = Vec::with_capacity(num_rounds as usize);
    let mut current_poly = polynomial.clone();

    for _ in 0..num_rounds {
        let proof_poly = get_round_partial_polynomial_proof(&current_poly.evaluation);

        transcript.append(&fq_vec_to_bytes(&proof_poly));

        proof_polynomials.push(proof_poly);

        let random_challenge = transcript.get_random_challenge();

        current_poly = current_poly.partial_evaluate(0, &random_challenge);
    }

    Proof {
        proof_polynomials,
        claimed_sum,
    }
}

pub fn verify(polynomial: &MultilinearPoly<Fq>, proof: Proof) -> bool {
    let mut transcript = Transcript::<Fq>::new();
    transcript.append(&fq_vec_to_bytes(&polynomial.evaluation));
    transcript.append(&fq_vec_to_bytes(&[proof.claimed_sum]));

    let mut current_poly = polynomial.clone();
    let mut random_challenges = Vec::with_capacity(proof.proof_polynomials.len());
    let mut expected_sum = proof.claimed_sum;

    for poly in proof.proof_polynomials {
        let poly = MultilinearPoly::new(poly.to_vec());

        if poly.evaluation.iter().sum::<Fq>() != expected_sum {
            return false;
        }

        transcript.append(&fq_vec_to_bytes(&poly.evaluation));
        let random_challenge = transcript.get_random_challenge();

        expected_sum =
            poly.evaluation[0] + random_challenge * (poly.evaluation[1] - poly.evaluation[0]);

        current_poly = current_poly.partial_evaluate(0, &random_challenge);

        random_challenges.push(random_challenge);
    }

    let poly_eval_sum = polynomial.evaluate(random_challenges);

    expected_sum == poly_eval_sum
}

////! verified incorrect
pub fn gkr_prove(
    claimed_sum: Fq,
    composed_polynomial: &SumPoly<Fq>,
    transcript: &mut Transcript<Fq>,
) -> GkrProof {
    let num_rounds = composed_polynomial.polys[0].evaluation[0].num_of_vars;
    let mut proof_polynomials = Vec::with_capacity(num_rounds as usize);
    let mut current_poly = composed_polynomial.clone();
    let mut random_challenges = Vec::new();

    for _ in 0..num_rounds {
        let proof_poly = get_round_partial_polynomial_proof_gkr(&current_poly); //this is f(b) then f(c)

        transcript.append(&fq_vec_to_bytes(&proof_poly.coefficient));

        proof_polynomials.push(proof_poly);

        let random_challenge = transcript.get_random_challenge(); //this is b and c aka r1 r2

        random_challenges.push(random_challenge);

        current_poly = current_poly.partial_evaluate(&random_challenge);
    }

    GkrProof {
        proof_polynomials,
        claimed_sum,
        random_challenges,
    }
}

pub fn gkr_verify(
    round_polys: Vec<UnivariatePoly<Fq>>,
    mut claimed_sum: Fq,
    transcript: &mut Transcript<Fq>,
) -> GkrVerify {
    let mut random_challenges = Vec::new();

    for round_poly in round_polys {
        let f_b_0 = round_poly.evaluate(Fq::from(0));
        let f_b_1 = round_poly.evaluate(Fq::from(1));

        if f_b_0 + f_b_1 != claimed_sum {
            return GkrVerify {
                verified: false,
                final_claimed_sum: Fq::from(0),
                random_challenges: vec![Fq::from(0)],
            };
        }

        transcript.append(&fq_vec_to_bytes(&round_poly.coefficient));

        let r_c = transcript.get_random_challenge();

        random_challenges.push(r_c);

        claimed_sum = round_poly.evaluate(r_c); //next expected sum
    }

    GkrVerify {
        verified: true,
        final_claimed_sum: claimed_sum,
        random_challenges,
    }
}

fn get_round_partial_polynomial_proof_gkr(composed_poly: &SumPoly<Fq>) -> UnivariatePoly<Fq> {
    let degree = composed_poly.get_degree();
    let mut poly_proof = Vec::with_capacity(degree + 1);

    for i in 0..=degree {
        let value = Fq::from(i as u64);
        let partial_poly = composed_poly.partial_evaluate(&value);
        let eval = partial_poly.reduce().iter().sum();
        poly_proof.push(eval);
    }

    let points = poly_proof
        .iter()
        .enumerate()
        .map(|(i, y)| (Fq::from(i as u64), *y))
        .collect();

    UnivariatePoly::interpolate(points)
}

fn get_round_partial_polynomial_proof(polynomial: &[Fq]) -> Vec<Fq> {
    let mid_point = polynomial.len() / 2;
    let (zeros, ones) = polynomial.split_at(mid_point);

    let poly_proof = vec![zeros.iter().sum(), ones.iter().sum()];

    poly_proof
}

#[cfg(test)]
mod test {
    use ark_bn254::Fq;
    use multilinear_polynomial::{
        composed_polynomial::{ProductPoly, SumPoly},
        multilinear_polynomial_evaluation::MultilinearPoly,
    };
    use univariate_polynomial::univariate_polynomial_dense::UnivariatePoly;

    use crate::sum_check_protocol::{prove, verify, Proof};

    use super::get_round_partial_polynomial_proof_gkr;

    #[test]
    fn test_valid_proving_and_verification() {
        let initial_polynomial = MultilinearPoly::new(vec![
            Fq::from(0),
            Fq::from(0),
            Fq::from(0),
            Fq::from(2),
            Fq::from(0),
            Fq::from(10),
            Fq::from(0),
            Fq::from(17),
        ]);

        let proof = prove(&initial_polynomial);

        let is_verified = verify(&initial_polynomial, proof);

        assert_eq!(is_verified, true);
    }

    #[test]
    fn test_invalid_proof_doesnt_verify() {
        let initial_polynomial =
            MultilinearPoly::new(vec![Fq::from(0), Fq::from(3), Fq::from(2), Fq::from(5)]);

        let false_proof = Proof {
            claimed_sum: Fq::from(20),
            proof_polynomials: vec![
                vec![Fq::from(3), Fq::from(9)],
                vec![Fq::from(1), Fq::from(2)],
            ],
        };

        let is_verified = verify(&initial_polynomial, false_proof);

        assert_eq!(is_verified, false);
    }

    #[test]
    fn test_get_gkr_round_poly() {
        let eval_1 = vec![Fq::from(0), Fq::from(3), Fq::from(2), Fq::from(5)];
        let eval_2 = vec![Fq::from(0), Fq::from(6), Fq::from(4), Fq::from(10)];
        let eval_3 = vec![Fq::from(0), Fq::from(1), Fq::from(1), Fq::from(2)];
        let eval_4 = vec![Fq::from(0), Fq::from(2), Fq::from(2), Fq::from(4)];

        let product_poly_1 = ProductPoly::new(vec![eval_1, eval_2]);
        let product_poly_2 = ProductPoly::new(vec![eval_3, eval_4]);

        let sum_poly = SumPoly::new(vec![product_poly_1, product_poly_2]);

        let points = vec![
            (Fq::from(0), Fq::from(20)),
            (Fq::from(1), Fq::from(68)),
            (Fq::from(2), Fq::from(156)),
        ];

        let expected_round_poly = UnivariatePoly::interpolate(points);

        let round_poly = get_round_partial_polynomial_proof_gkr(&sum_poly);

        assert_eq!(round_poly.coefficient, expected_round_poly.coefficient);
    }
}
